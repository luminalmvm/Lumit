// One row of the Project panel — the tree's line for a single item — and the
// two small pieces that only a row draws: the state badge and the label that
// follows the pointer while a row is dragged.
//
// **In plain terms**: a row is a twirl, a type mark, a name, whatever badges
// the item has earned, and the metadata columns. It is handed everything it
// draws by the panel's own walk, so hovering one row costs nothing at the
// bridge (the budget test expects zero).

import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../state/drag_payloads.dart';
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/share_marks.dart';
import '../widgets/controls.dart';
import 'project_columns_frb.dart';
import 'project_menu_frb.dart';
import 'timeline_extras_frb.dart' show showLabelPicker;

/// The state badge ("missing"): 14 tall, 4px of padding either side, its text
/// mono at 9 with no tracking — a badge is not a container label, so it is not
/// a kicker however small it is.
const double _badgeHeight = 14;
const double _badgePad = 4;
const double _badgeTextSize = 9;

/// The badge's outline is its own text colour, hushed: the mockup's border
/// resolves to that colour at 28% over the panel, on both badges.
const double _badgeBorderAlpha = 0.28;

/// The dot for each other person who has a composition open in a shared
/// project, the Timeline's label dot's own size.
const double _personDotSize = personDiscSize;

/// How many people a composition's row marks before the last is a count,
/// and how much wider than a disc that count can be.
const int _rowPeople = 3;
const double _countSpare = 8;

/// The least room a row's name keeps, about twelve letters, whatever badges
/// the row wears.
const double _nameFloor = 96;

/// The row's label square: an 8px hue-quartered mark in a 14px slot —
/// the slot is the hit target, and its width is fixed so the squares
/// stand in a column however deep the rows are indented.

/// The label chip an untagged item's kind wears by default — the mockup's own
/// per-type tints, which are the label palette's chips: azure for
/// picture footage, indigo for sound, amber for solids. Folders and
/// compositions stay muted, so they wear no chip at all (`0`).
///
/// One function because two places must agree on it: the row's glyph draws
/// this colour, and the panel's swatch filter matches on it — the colour a
/// fresh item is *wearing* is an answer the filter has to honour, and
/// the two used to disagree, which made a just-imported clip invisible to the
/// very colour it was showing.
int projectDefaultLabel(ItemReference item, {required bool audio}) =>
    switch (item) {
      ItemReference_Footage() => audio ? 6 : 1,
      ItemReference_Solid() => 2,
      ItemReference_Folder() || ItemReference_Composition() => 0,
    };

/// What a click on a row does to the selection.
enum SelectMode {
  /// Plain click: this row, and only this row.
  replace,

  /// `Ctrl` (or `Cmd`): add this row, or drop it if it was already in.
  toggle,

  /// `Shift`: every row between the anchor and this one.
  range,
}

/// The modifier held right now, as a selection rule. Read from the keyboard
/// rather than carried on the tap because `GestureDetector.onTap` does not
/// report modifiers.
SelectMode _selectModeFromKeyboard() {
  final keys = HardwareKeyboard.instance.logicalKeysPressed;
  bool down(LogicalKeyboardKey a, LogicalKeyboardKey b) =>
      keys.contains(a) || keys.contains(b);
  if (down(LogicalKeyboardKey.shiftLeft, LogicalKeyboardKey.shiftRight)) {
    return SelectMode.range;
  }
  if (down(LogicalKeyboardKey.controlLeft, LogicalKeyboardKey.controlRight) ||
      down(LogicalKeyboardKey.metaLeft, LogicalKeyboardKey.metaRight)) {
    return SelectMode.toggle;
  }
  return SelectMode.replace;
}

/// A state badge: the mockup's small outlined pill, in its own colour hushed
/// to an outline. Not a kicker — a kicker names a container, and this reports
/// a state — so it is plain mono with no tracking and no capitals.
class ProjectBadge extends StatelessWidget {
  final String label;
  final Color colour;

  const ProjectBadge({super.key, required this.label, required this.colour});

  /// How wide a badge saying [label] is drawn, edge to edge.
  static double widthOf(LumitTheme t, String label, TextScaler scale) =>
      _lineWidth(label, t.mono.copyWith(fontSize: _badgeTextSize), scale) +
      2 * (_badgePad + 1);

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      height: _badgeHeight,
      alignment: Alignment.center,
      padding: const EdgeInsets.symmetric(horizontal: _badgePad),
      decoration: BoxDecoration(
        border: Border.all(color: colour.withValues(alpha: _badgeBorderAlpha)),
        borderRadius: BorderRadius.circular(t.tokens.actionRadius),
      ),
      child: Text(
        label,
        style: t.mono.copyWith(fontSize: _badgeTextSize, color: colour),
      ),
    );
  }
}

/// How wide [text] is on one line. Kept once measured, since rows ask on
/// every build and a panel holds few different names and fewer badges.
double _lineWidth(String text, TextStyle style, TextScaler scale) {
  final key = '$text|${style.fontFamily}|${scale.scale(style.fontSize ?? 0)}';
  // ponytail: emptied when it has grown, not trimmed. A project with more
  // than a few hundred names on screen at once would want the oldest out.
  if (_lineWidths.length > 512) _lineWidths.clear();
  return _lineWidths[key] ??= () {
    final words = TextPainter(
      text: TextSpan(text: text, style: style),
      textDirection: TextDirection.ltr,
      textScaler: scale,
      maxLines: 1,
    )..layout();
    final width = words.width;
    words.dispose();
    return width;
  }();
}

final Map<String, double> _lineWidths = {};

/// Relink: ask which file [footage] is now, and point it there. The files
/// that moved with it are found from the one. False when none was picked.
Future<bool> relinkFootage(FootageReference footage,
    {Future<String?> Function()? picker}) async {
  final path = picker != null
      ? await picker()
      : await pickFootage()
          .then((paths) => paths.isEmpty ? null : paths.first);
  if (path == null) return false;
  footage.relink(path: path);
  return true;
}

/// One Project panel row.
class ProjectRowFrb extends StatefulWidget {
  final ItemReference item;

  /// The item's name, read once by the panel's walk. Passed in rather than
  /// fetched here because this row rebuilds on every hover flicker, and a
  /// bridge call per hover was exactly the chatter Airyzz measured.
  final String name;
  final int depth;
  final bool missing;

  /// On disk, but the engine cannot decode it.
  final bool undecodable;

  /// Sound with no picture — the media probe's own answer, which is what picks
  /// the speaker glyph over the film one.
  final bool audio;

  /// This item's colour tag, an index into the label palette. `0` is untagged,
  /// and an untagged row keeps its per-type tint (§12A.3a: a tag **tints the
  /// glyph's strokes** rather than adding a dot beside it).
  final int label;

  /// The colour the containing folders hand down — the nearest ancestor folder
  /// with a tag, or `0` where none of them has one (A12). It tints this row's
  /// glyph only while the row has no tag of its own: an explicit colour
  /// overrides what it inherits.
  final int inherited;

  /// Whether this composition is a **node graph**: boxes and wires in place of
  /// a layer stack (docs/impl/node-graph-comp.md §4.4). It changes the glyph
  /// and nothing else: a node graph opens, drags and nests as any comp does.
  /// Handed in from the panel's cache, for the reason the name is.
  final bool nodeGraph;

  /// Whether any composition places this item — the `in use` badge.
  final bool inUse;

  /// This item's proxy, or null where it has none. Null on every kind
  /// but footage — nothing else has a media reference to stand in for.
  final BridgeProxy? proxy;

  /// This footage is read from a stand-in another person sent.
  final bool standIn;

  /// The other people in a shared project who have this composition open,
  /// each as their colour, their name and the letters their disc carries.
  /// Empty on every other kind of row.
  final List<(int, String, String)> people;

  /// They are only what was last heard, and are drawn fainter.
  final bool peopleStale;

  /// The colour of each other person in a shared project who has this item
  /// selected. The row carries a bar of them. Empty whenever nobody has.
  final List<int> holders;
  final bool selected;

  /// Whether this is one of the alternate rows, which stand on a slightly
  /// different ground so the eye can follow a line across the columns.
  final bool shaded;
  final bool renaming;

  /// Which optional columns this width carries, and the finished strings to put
  /// in them — both worked out by the panel, for the same reason the name is.
  final ProjectColumns columns;
  final ProjectCells cells;

  /// Whether the rows carry poster frames (Settings, **Thumbnails on footage
  /// rows**). On, every row's glyph slot takes [projectRowThumbWidth], so the
  /// names still stand in one column whatever kind of row is above or below.
  final bool thumbnails;

  /// This footage item's poster frame, out of the panel's own cache, or null
  /// while it is decoding, for a file with no picture, and for every other
  /// kind of row. Only read while [thumbnails] is on.
  final ui.Image? thumb;

  /// Whether this row is the whole selection, which is what decides on a click
  /// that there is nothing to collapse.
  ///
  /// The count of picked rows would say the same thing, and the row would then
  /// have to be rebuilt every time the count moved anywhere in the panel. This
  /// is the row's own answer, so a click redraws the two rows whose shading
  /// changed.
  final bool loneSelection;
  final ValueChanged<SelectMode> onSelect;

  /// The panel's whole footage selection, read when a drag starts so dragging
  /// any selected row brings the rest with it.
  final List<FootageReference> Function() selectedFootage;
  final VoidCallback onStartRename;
  final VoidCallback onEndRename;
  final VoidCallback onFindMissing;

  /// Make a comp from these items — the panel's own New composition, so a
  /// double-clicked footage item goes through the one funnel every other route
  /// does.
  final void Function(List<FootageReference>) onNewComposition;

  /// Whether this row's folder is showing its children, and the toggle that
  /// opens or shuts it. Meaningless on every other kind.
  final bool folderOpen;
  final VoidCallback onToggleFolder;

  /// Called after an edit this row made, so the panel re-reads at once rather
  /// than waiting for the engine's change stream to come back around.
  final VoidCallback onLocalEdit;

  /// Tag this row — and the rest of the selection, when this row is part of
  /// one — from the context menu's chip strip.
  final ValueChanged<int> onSetLabel;

  /// File these items into this row's folder — the drop gesture. Null on every
  /// kind but a folder, which is what makes a folder row the only drop target.
  final void Function(List<ItemReference> items)? onDropItems;

  /// File this row (and the rest of the selection, when it is part of one) into
  /// the folder picked from **Move to folder**.
  final void Function(ItemReference folder) onMoveToFolder;

  /// The folders **Move to folder** may offer, read when the menu is raised.
  /// A function rather than a list because a menu is a gesture: reading the
  /// tree per rebuild is exactly the chatter the budget test guards against.
  final List<(String, ItemReference)> Function() folderChoices;

  /// What a command from this row's menu acts on — the panel's own
  /// `_targets`, which is the whole selection when this row is in it and this
  /// row alone when it is not. A function for the same reason
  /// [folderChoices] is: it is read when the menu is raised, never in build.
  final List<ItemReference> Function() menuTargets;
  final Future<String?> Function()? relinkPicker;

  const ProjectRowFrb({
    super.key,
    required this.item,
    required this.name,
    required this.depth,
    required this.missing,
    this.undecodable = false,
    required this.audio,
    required this.label,
    required this.inherited,
    this.nodeGraph = false,
    required this.inUse,
    required this.proxy,
    this.standIn = false,
    this.people = const [],
    this.peopleStale = false,
    this.holders = const [],
    required this.selected,
    this.shaded = false,
    required this.renaming,
    required this.columns,
    required this.cells,
    this.thumbnails = false,
    this.thumb,
    required this.loneSelection,
    required this.onSelect,
    required this.selectedFootage,
    required this.onStartRename,
    required this.onEndRename,
    required this.onFindMissing,
    required this.onNewComposition,
    required this.folderOpen,
    required this.onToggleFolder,
    required this.onLocalEdit,
    required this.onSetLabel,
    required this.onMoveToFolder,
    required this.menuTargets,
    required this.folderChoices,
    this.onDropItems,
    this.relinkPicker,
  });

  @override
  State<ProjectRowFrb> createState() => _ProjectRowFrbState();
}

class _ProjectRowFrbState extends State<ProjectRowFrb> {
  bool _hover = false;
  TextEditingController? _rename;
  // Escape on the field's own node, ahead of the shortcut system: the
  // row's editor is a bare EditableText rather than a HouseTextField, so it
  // wires the same key itself instead of inheriting it.
  late final FocusNode _renameFocus = FocusNode(onKeyEvent: _onRenameKey);

  KeyEventResult _onRenameKey(FocusNode node, KeyEvent event) {
    if (event is KeyDownEvent &&
        event.logicalKey == LogicalKeyboardKey.escape) {
      _cancelRename();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  ItemReference get item => widget.item;

  @override
  void dispose() {
    _rename?.dispose();
    _renameFocus.dispose();
    super.dispose();
  }

  @override
  void didUpdateWidget(ProjectRowFrb old) {
    super.didUpdateWidget(old);
    if (widget.renaming && !old.renaming) {
      _rename = TextEditingController(text: widget.name);
      _renameFocus.requestFocus();
    }
  }

  /// Escape: shut the editor, rename nothing.
  void _cancelRename() {
    _rename?.dispose();
    _rename = null;
    widget.onEndRename();
  }

  void _commitRename() {
    final text = _rename?.text.trim() ?? '';
    if (text.isNotEmpty && text != widget.name) {
      item.rename(name: text);
      widget.onLocalEdit();
    }
    _rename?.dispose();
    _rename = null;
    widget.onEndRename();
  }

  /// Whether this row was already selected when the pointer went down — what
  /// decides, at mouse-up, between renaming and collapsing the selection.
  bool _wasSelectedAtDown = false;

  /// The click in flight: where it started, whether it wandered past the
  /// touch slop (a drag, not a click), and whether it was the primary button.
  Offset _downAt = Offset.zero;
  bool _dragged = false;
  bool _primaryDown = false;

  /// Selection happens on pointer DOWN, not on the resolved tap: the tap
  /// gesture waits out the double-click window and the drag arena, which read
  /// as the panel lagging behind the mouse. The one case that must wait is a
  /// plain press on an already-selected row — collapsing a multi-selection on
  /// the down stroke would make dragging that selection impossible.
  void _handlePointerDown(PointerDownEvent event) {
    if (event.buttons != kPrimaryButton) return;
    _primaryDown = true;
    _downAt = event.position;
    _dragged = false;
    _wasSelectedAtDown = widget.selected;
    final mode = _selectModeFromKeyboard();
    if (mode == SelectMode.replace && widget.selected) return;
    widget.onSelect(mode);
  }

  void _handlePointerMove(PointerMoveEvent event) {
    if ((event.position - _downAt).distance > kTouchSlop) _dragged = true;
  }

  /// The click, resolved on the raw pointer UP rather than through the
  /// gesture arena — the arena waits out the empty-area double-tap window,
  /// which is exactly the lag being avoided.
  ///
  /// All it settles is the one thing the down stroke could not: **a plain
  /// click on one row of a multi-selection collapses the selection to it**.
  /// A click on a row that is already the only one selected does nothing at
  /// all, which is what clicking something already chosen means everywhere
  /// else in the application. It used to *open* the row — and because
  /// this is the raw pointer-up, "the second click of a double-click" and "a
  /// click on a row selected a minute ago" were the same event: selecting a
  /// clip and clicking it again raised New composition. That is exactly the
  /// mistake click-to-rename was taken out for, made a second time under
  /// another name. Opening is [_open], on the double-tap.
  void _handlePointerUp(PointerUpEvent event) {
    if (!_primaryDown) return;
    _primaryDown = false;
    if (_dragged || !_wasSelectedAtDown) return;
    if (_selectModeFromKeyboard() != SelectMode.replace) return;
    if (!widget.loneSelection) widget.onSelect(SelectMode.replace);
  }

  /// **Opening a row**, and what opening means is the item's own answer: a
  /// composition fronts in the Timeline, footage raises New composition
  /// sized and timed to it (or loads into the Viewer's source side when one
  /// is on screen), a folder opens and shuts.
  ///
  /// On the double-tap, which is the gesture it always meant — the row's own
  /// recogniser, which fires on the second click's *up* rather than waiting a
  /// further window, so nothing about the speed of it changed. Selection is
  /// still on the down stroke, so the pair still reads as "select, then open"
  /// in one motion.
  void _open() {
    if (widget.renaming) return;
    if (item case ItemReference_Composition(:final field0)) {
      Provider.of<LumitUiState>(context, listen: false).setSelectedComp(field0);
      return;
    }
    // Footage has no window of its own to open, and the thing people want
    // from a clip they have just double-clicked is a comp to put it in —
    // already the size, rate and length of the media, because that dialogue
    // reads the selection (the longest item wins when there are several).
    if (item case ItemReference_Footage(:final field0)) {
      // With a source side on screen, as the Cut workspace has, the clip
      // loads into it to be marked and placed.
      final ui = Provider.of<LumitUiState>(context, listen: false);
      if (ui.hasSourceView) {
        ui.openFootageView(field0);
        return;
      }
      final selected = widget.selectedFootage();
      widget.onNewComposition(selected.isEmpty ? [field0] : selected);
      return;
    }
    if (item is ItemReference_Folder) widget.onToggleFolder();
    // Nothing for the other kinds. Renaming is `Enter` on the selection, with
    // the row menu's Rename as the mouse path.
  }

  Future<void> _doRelink(FootageReference footage) async {
    if (await relinkFootage(footage, picker: widget.relinkPicker)) {
      widget.onLocalEdit();
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final row = MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      // A raw listener, not a gesture: down and up fire the instant they
      // happen, without waiting for the tap/drag/double-tap arena to resolve.
      child: Listener(
        onPointerDown: _handlePointerDown,
        onPointerMove: _handlePointerMove,
        onPointerUp: _handlePointerUp,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          // **This is where opening a row happens**. It also claims
          // double-clicks on the row in the gesture arena, so the panel's
          // empty-area double-tap (import) never fires for a double-click on
          // an item — which is why it was registered even while it did
          // nothing.
          onDoubleTap: _open,
          onSecondaryTapDown: (d) {
            // A right-click on a row already in the selection keeps it: the menu
            // is about what is picked, and collapsing four rows to one because the
            // menu was opened would throw the selection away.
            if (!widget.selected) widget.onSelect(SelectMode.replace);
            showProjectMenuFrb(
              context: context,
              item: item,
              missing: widget.missing,
              position: d.globalPosition,
              onFindMissing: widget.onFindMissing,
              onLocalEdit: widget.onLocalEdit,
              onStartRename: widget.onStartRename,
              onSetLabel: widget.onSetLabel,
              label: widget.label,
              inherited: widget.inherited,
              onRelink: item is ItemReference_Footage
                  ? () => _doRelink((item as ItemReference_Footage).field0)
                  : null,
              proxy: widget.proxy,
              // The same picker seam the relink uses — picking a proxy file
              // and picking a replacement are the same gesture over the same
              // dialogue, and tests inject one stub for both.
              proxyPicker: widget.relinkPicker,
              // Read here, not in build: raising the menu is a gesture.
              folders: widget.folderChoices(),
              onMoveToFolder: widget.onMoveToFolder,
              targets: widget.menuTargets(),
            );
          },
          child: Container(
            height: projectRowHeight,
            // Three greys at rest (§2.1): the row's own selected fill is
            // the header grey, and `surface_3` appears only under the pointer.
            // An alternate row takes half a step towards the selected fill,
            // so a picked or hovered row still stands clear of both grounds.
            color: widget.selected
                ? t.surface2
                : _hover
                    ? t.surface3
                    : widget.shaded
                        ? t.surface2.withValues(alpha: 0.5)
                        : null,
            foregroundDecoration: widget.holders.isEmpty
                ? null
                : TheirBar(
                    [for (final c in widget.holders) t.personColour(c)],
                    tint: 0.14,
                  ),
            padding: EdgeInsets.only(
              left: projectRowPadding + widget.depth * projectIndentPerDepth,
              right: projectRowPadding,
            ),
            child: Row(
              children: [
                _twirl(t),
                const SizedBox(width: projectRowGap),
                _glyph(t),
                const SizedBox(width: projectRowGap),
                // The row's flexible slot, and so the Name column: everything
                // to its right is a fixed box and the last of those swallows
                // the panel's spare width, so what is left here settles at
                // [projectNameColumn] however wide the panel grows (owner,
                // desk test). The indent and the badges come out of it exactly
                // as they always did.
                Expanded(child: _nameAndBadges(t)),
                if (widget.missing) ...[
                  const SizedBox(width: projectRowGap),
                  // The badge *is* the relink control: the mockup gives a
                  // broken row a pill and no button, and the panel would
                  // otherwise lose the one-click relink the row has always had.
                  LumitTooltip(
                    message: l10n.relinkEllipsis,
                    child: HoverMark(
                      swells: true,
                      builder: (context, lit) => GestureDetector(
                        key: ValueKey<String>('relink-${projectItemId(item)}'),
                        behavior: HitTestBehavior.opaque,
                        onTap: () =>
                            _doRelink((item as ItemReference_Footage).field0),
                        // Claims a double-click here too, so the row never
                        // opens New composition over the badge.
                        onDoubleTap: () =>
                            _doRelink((item as ItemReference_Footage).field0),
                        child: ProjectBadge(
                          label: l10n.projectItemMissing,
                          colour:
                              Color.lerp(t.warning, t.textPrimary, 0.35 * lit)!,
                        ),
                      ),
                    ),
                  ),
                ],
                if (widget.undecodable) ...[
                  const SizedBox(width: projectRowGap),
                  LumitTooltip(
                    message: l10n.tipProjectUndecodable,
                    child: ProjectBadge(
                      key: ValueKey<String>(
                          'undecodable-${projectItemId(item)}'),
                      label: l10n.projectItemUndecodable,
                      colour: t.warning,
                    ),
                  ),
                ],
                // **The row's own label square** (the owner's ask): a
                // hue-quartered mark that opens the eight-colour picker to
                // tag this row — and the rest of the selection when the row
                // is part of one, the same reach the menu's chip strip has.
                // At the row's right, before the metadata columns, so the
                // squares stand in their own column and the mockup's name
                // cluster keeps its measured places. No gap of its own: the
                // slot's inset either side of the 8px square is the standoff,
                // and the first cell brings the usual row gap with it.
                ...widget.columns.cells(
                  items: widget.cells.items,
                  size: widget.cells.size,
                  fps: widget.cells.fps,
                  duration: widget.cells.duration,
                  path: widget.cells.path,
                  style: projectMetaStyle(t),
                  pathStyle: projectMetaStyle(t).copyWith(color: t.textDisabled),
                ),
              ],
            ),
          ),
        ),
      ),
    );

    // Footage drags onto the Timeline (or onto New composition) as
    // `FootageDragData`; a composition drags onto the Timeline alone, as
    // `CompDragData`, to nest as a Precomp layer. The payload types are the
    // contract — the drop targets consume exactly these.
    if (item case ItemReference_Footage(:final field0)) {
      final name = widget.name;
      // Dragging a row that is part of the selection brings the whole selection;
      // dragging an unselected row is about that row alone, which is what every
      // file list does and what stops a stale selection following the pointer.
      final selection = widget.selected ? widget.selectedFootage() : const [];
      final dragged = selection.length > 1
          ? List<FootageReference>.from(selection)
          : <FootageReference>[field0];
      return Draggable<FootageDragData>(
        data: FootageDragData(
          dragged,
          dragged.length > 1 ? '${dragged.length} items' : name,
        ),
        dragAnchorStrategy: pointerDragAnchorStrategy,
        feedback: _DragFeedbackFrb(
          name: dragged.length > 1 ? '${dragged.length} items' : name,
        ),
        child: row,
      );
    }
    if (item case ItemReference_Composition(:final field0)) {
      return Draggable<CompDragData>(
        data: CompDragData(field0, widget.name),
        dragAnchorStrategy: pointerDragAnchorStrategy,
        feedback: _DragFeedbackFrb(name: widget.name, icon: LumitIcon.comp),
        child: row,
      );
    }
    // A folder row takes what the other rows drag: dropping on it files the
    // items there. Two nested targets rather than one, because a
    // `DragTarget` is typed and the panel's rows carry two payload types — the
    // same two the Timeline consumes.
    final drop = widget.onDropItems;
    if (drop != null) {
      return DragTarget<FootageDragData>(
        onAcceptWithDetails: (d) =>
            drop([for (final f in d.data.footage) ItemReference.footage(f)]),
        builder: (context, footageOver, _) => DragTarget<CompDragData>(
          onAcceptWithDetails: (d) =>
              drop([ItemReference.composition(d.data.comp)]),
          builder: (context, compOver, _) => Container(
            // The drop-target treatment (§6.5), painted over the row rather
            // than behind it: the row draws its own fill, and a drop with no
            // feedback is indistinguishable from one that did nothing.
            foregroundDecoration: footageOver.isEmpty && compOver.isEmpty
                ? null
                : BoxDecoration(
                    border: Border.all(color: t.accent, width: 1.5),
                    color: t.accent.withValues(alpha: 0.1),
                  ),
            child: row,
          ),
        ),
      );
    }
    return row;
  }

  /// The twirl's slot. A shut folder has to say so, or it reads as an empty
  /// one; the caret is its own target as well, the way the Hierarchy's is —
  /// and every row keeps the slot whether or not it has one, so a child still
  /// lines up one indent step right of the folder holding it.
  Widget _twirl(LumitTheme t) => SizedBox(
        width: projectRowIconSize,
        child: item is! ItemReference_Folder
            ? null
            : HoverMark(
                builder: (context, lit) => GestureDetector(
                  key:
                      ValueKey<String>('project-twirl-${projectItemId(item)}'),
                  behavior: HitTestBehavior.opaque,
                  onTap: widget.onToggleFolder,
                  child: TwirlTurn(
                    open: widget.folderOpen,
                    child: lumitIcon(
                      widget.folderOpen
                          ? LumitIcon.twirlOpen
                          : LumitIcon.twirlClosed,
                      size: projectRowIconSize,
                      color: Color.lerp(t.textMuted, t.textPrimary, lit)!,
                    ),
                  ),
                ),
              ),
      );

  /// The row's type glyph, tinted by what the item *is* — the mockup's own
  /// per-type tints, which are the label palette's own chips: azure for
  /// picture footage, indigo for sound, amber for solids. A folder and a
  /// composition stay muted, as the mockup draws them, and missing footage
  /// wears the warning-tinted unlink glyph. No thumbnail here by default: the
  /// preview card carries the picture, so the tree stays a tight list of
  /// names. With **Thumbnails on footage rows** switched on, a footage row's
  /// poster frame stands in the slot instead, the row's own height tall; the
  /// glyph stays for a file with no picture, a missing one, and a frame that
  /// has not decoded yet.
  ///
  /// **A colour tag takes the glyph over** (docs/07 §3.1, §12A.3a: the tag
  /// tints the icon's strokes rather than adding a dot). The per-type tint is
  /// what an *untagged* item wears — a default, not a fact — so a tag that
  /// left it alone would need a mark of its own, which is what the mockup
  /// deliberately does not draw. Missing still wins over both: a file that is
  /// not there is more urgent than what colour someone filed it under.
  ///
  /// **A folder hands its colour down** (A12): a row with no tag of its own
  /// wears the nearest tagged ancestor folder's, and only an item with neither
  /// falls back to the per-type tint. An explicit tag always wins over what is
  /// inherited, which is what makes tagging one clip inside a tagged folder
  /// mean something.
  Widget _glyph(LumitTheme t) {
    final (icon, tint) = _iconFor(item, t);
    final tag = widget.label != 0 ? widget.label : widget.inherited;
    final thumb = widget.thumbnails && !widget.missing ? widget.thumb : null;
    // The glyph is also the colour control: it already wears the label
    // colour, so pressing it is where a person looks to change it.
    return KeyedSubtree(
      key: ValueKey<String>('project-glyph-${projectItemId(item)}'),
      child: LumitTooltip(
        message: l10n.tipLabelColour,
        child: HoverMark(
          builder: (_, lit) => GestureDetector(
            key:
                ValueKey<String>('project-label-swatch-${projectItemId(item)}'),
            behavior: HitTestBehavior.opaque,
            onTapDown: (d) async {
              final picked = await showLabelPicker(context, d.globalPosition,
                  keyPrefix: 'project-label');
              if (picked != null) widget.onSetLabel(picked);
            },
            child: SizedBox(
              width: widget.thumbnails
                  ? projectRowThumbWidth
                  : projectRowIconSize,
              height: projectRowHeight,
              child: thumb != null
                  // The card's own treatment, at the row's size: the frame
                  // fills its slot and the slot wears the control corner. A
                  // pixel clear above and below, so the frames of rows that
                  // follow one another do not run into one strip.
                  ? Padding(
                      padding: const EdgeInsets.symmetric(vertical: 1),
                      child: ClipRRect(
                        borderRadius:
                            BorderRadius.circular(t.tokens.controlRadius),
                        child: Container(
                          color: t.surface0,
                          child: RawImage(image: thumb, fit: BoxFit.cover),
                        ),
                      ),
                    )
                  : Center(
                      // The tint says what the item is, so it is lifted and
                      // not replaced.
                      child: lumitIcon(
                        widget.missing ? LumitIcon.unlink : icon,
                        size: projectRowIconSize,
                        color: Color.lerp(
                            widget.missing
                                ? t.warning
                                : tag != 0
                                    ? t.labelColour(tag)
                                    : tint,
                            t.textPrimary,
                            0.35 * lit)!,
                      ),
                    ),
            ),
          ),
        ),
      ),
    );
  }

  /// The name, then who else has the composition open, then the quiet
  /// badges: placed somewhere, read from a proxy, read from a stand-in.
  ///
  /// **The name keeps [_nameFloor] of room, or all it needs when that is
  /// less.** It is the one thing on the row that cannot be guessed, and two
  /// badges used to crush it to three letters. So the badges take what is
  /// left past that, and the ones that do not fit are left out, the last
  /// first. What a row is stays on the name's tooltip, badge or no badge.
  /// The badges that want acting on, missing and undecodable, are not among
  /// these and are never left out.
  Widget _nameAndBadges(LumitTheme t) {
    final id = projectItemId(item);
    final badges = [
      // Placed somewhere (§12A.3a). Before the missing badge, so a broken
      // file that is nonetheless *in* a comp reads left to right as "used,
      // and lost", which is the order those two facts matter in.
      if (widget.inUse) ('in-use-$id', l10n.projectItemInUse, t.success),
      // Reading from its proxy. **Quiet on purpose**: a proxy is a fact
      // about which file the item is being read from, not something that
      // wants acting on, so it takes the badge family's shape at
      // `text_muted` and claims none of the palette. Drawn only while the
      // tick is on: a proxy that is attached and switched off has nothing
      // to announce.
      if (widget.proxy?.inUse ?? false)
        ('proxy-$id', l10n.projectItemProxy, t.textMuted),
      // Read from a stand-in someone in the shared project sent, the file
      // itself not being on this computer. Quiet like the proxy's, and for
      // its reason.
      if (widget.standIn)
        ('stand-in-$id', l10n.projectItemStandIn, t.textMuted),
    ];
    final name = _nameOrEditor(t);
    if (badges.isEmpty && widget.people.isEmpty) return name;
    // Who else has this composition open, a dot each in their colour. The
    // name is the tooltip.
    final scale = MediaQuery.textScalerOf(context);
    return LayoutBuilder(
      builder: (context, box) {
        // What is left past the name's floor goes to the people first, as
        // many as fit and never fewer than a count of them, and then to the
        // badges, in order, until one does not fit.
        final needs = widget.renaming
            ? _nameFloor
            : math.min(_nameFloor, _lineWidth(widget.name, t.body, scale));
        var room = box.maxWidth - needs;
        const each = projectRowGap + _personDotSize;
        final marks =
            ((room - _countSpare) / each).floor().clamp(1, _rowPeople);
        final drawn = peopleDrawn(widget.people.length, marks);
        final more = widget.people.length - drawn;
        if (widget.people.isNotEmpty) {
          room -= drawn * each + (more > 0 ? each + _countSpare : 0);
        }
        final fit = badges.takeWhile((badge) {
          room -= projectRowGap + ProjectBadge.widthOf(t, badge.$2, scale);
          return room >= 0;
        }).toList();
        return Row(
          children: [
            Expanded(
              child: badges.isEmpty
                  ? name
                  : LumitTooltip(
                      message: [
                        widget.name,
                        for (final badge in badges) badge.$2
                      ].join(' · '),
                      child: name,
                    ),
            ),
            for (final (colour, name, letters)
                in widget.people.take(drawn)) ...[
              const SizedBox(width: projectRowGap),
              LumitTooltip(
                message: name,
                child: PersonDisc(
                    colour: colour,
                    name: name,
                    letters: letters,
                    stale: widget.peopleStale),
              ),
            ],
            if (more > 0) ...[
              const SizedBox(width: projectRowGap),
              LumitTooltip(
                message:
                    widget.people.skip(drawn).map((p) => p.$2).join(', '),
                child: MorePeople(count: more),
              ),
            ],
            for (final (key, label, colour) in fit) ...[
              const SizedBox(width: projectRowGap),
              ProjectBadge(
                key: ValueKey<String>(key),
                label: label,
                colour: colour,
              ),
            ],
          ],
        );
      },
    );
  }

  /// The label square: always the hues, never the row's current colour — the
  /// glyph two cells left already wears that, so this stays the mark that
  /// says "a colour is set here" rather than a second copy of the answer.
  /// Opens the same eight-colour picker every label control does.
  Widget _nameOrEditor(LumitTheme t) {
    final controller = _rename;
    if (widget.renaming && controller != null) {
      return Container(
        padding: const EdgeInsets.symmetric(horizontal: 4),
        decoration: BoxDecoration(
          color: t.surface0,
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          border: Border.all(color: t.accent),
        ),
        child: EditableText(
          key: const ValueKey('rename-field'),
          controller: controller,
          focusNode: _renameFocus,
          style: t.body,
          cursorColor: t.accent,
          backgroundCursorColor: t.surface2,
          selectionColor: t.accent.withValues(alpha: 0.5),
          onSubmitted: (_) => _commitRename(),
          onTapOutside: (_) => _commitRename(),
        ),
      );
    }
    // A folder names a group, and a picked row is the one being talked about:
    // both read at `text_primary`. A broken one drops to muted — the badge
    // beside it is what carries the news.
    final colour = widget.missing
        ? t.textMuted
        : (widget.selected || item is ItemReference_Folder)
            ? t.textPrimary
            : t.textSecondary;
    return Text(widget.name,
        style: t.body.copyWith(color: colour), overflow: TextOverflow.ellipsis);
  }

  /// The per-type glyph and tint, off [projectDefaultLabel] so the filter
  /// matches exactly what this draws. Sound has no picture — the media
  /// probe's own answer, not a guess at the file name — which is what tells
  /// the two footage tints apart.
  (LumitIcon, Color) _iconFor(ItemReference item, LumitTheme t) {
    final fallback = projectDefaultLabel(item, audio: widget.audio);
    return switch (item) {
      ItemReference_Footage() => (
          widget.audio ? LumitIcon.audioFile : LumitIcon.footage,
          t.labelColour(fallback),
        ),
      ItemReference_Folder() => (LumitIcon.folder, t.textMuted),
      // A node graph wears the boxes-and-wires glyph, muted as a comp's is:
      // it is a comp, and what differs is what is inside it.
      ItemReference_Composition() => (
          widget.nodeGraph ? LumitIcon.nodes : LumitIcon.comp,
          t.textMuted,
        ),
      ItemReference_Solid() => (LumitIcon.solid, t.labelColour(fallback)),
    };
  }
}

/// The floating label shown under the pointer while a row is dragged.
class _DragFeedbackFrb extends StatelessWidget {
  final String name;
  final LumitIcon icon;
  const _DragFeedbackFrb({required this.name, this.icon = LumitIcon.footage});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            lumitIcon(icon,
                size: iconSize,
                color: icon == LumitIcon.comp ? t.textMuted : t.labelColour(1)),
            const SizedBox(width: 6),
            Text(name, style: t.small),
          ],
        ),
      ),
    );
  }
}
