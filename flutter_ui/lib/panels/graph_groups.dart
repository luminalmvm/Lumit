// Groups and notes on a graph canvas.
//
// A group is a named, coloured frame behind some boxes. It stores which boxes
// and nothing about where, so the frame is drawn round wherever they sit and
// follows them. The bar along its top is the handle: a press picks every box
// in it, a drag moves them, a double-click renames it and a right-click opens
// its menu. Boxes let go over a frame move into it.
//
// Groups nest. One group is inside another when all of its boxes are the
// other's too, so nothing more is stored for it: the outer frame is drawn
// round the inner one, and every edit here keeps any two groups either apart
// or one inside the other.
//
// A note is a few words on the canvas. It is not a box: it has no sockets,
// nothing reads it and it is not part of the pick. It is dragged by its body,
// reworded on a double-click and has a menu on a right-click.
//
// The layer canvas and the node graph canvas hold different boxes and write
// different lists. Each tells [GraphGroups] what its groups and notes are and
// how to write them back, and the rest is here once.

import 'dart:convert' show jsonDecode;
import 'dart:io' show File;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart' show HardwareKeyboard, LogicalKeyboardKey;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart'
    show BridgePresetInfo, presetsDirPath, renameSavedGroup;
import 'package:lumit_flutter/src/rust/api/graph.dart' show BridgeGraphNote;
import 'package:lumit_flutter/src/rust/api/keymap.dart' show BridgeKeyContext;
import 'package:lumit_flutter/src/rust/api/shell.dart' show revealInFolder;

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/settings_account.dart' show askBefore;
import '../shell/theme_name_dialog.dart';
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'graph_panel.dart';
import 'timeline_extras_frb.dart' show DoubleTap, showLabelPicker, showMenuAt;

/// One group as a canvas works on it: its boxes by key.
class GraphGroup {
  final String name;

  /// Which chip of the label palette tints it.
  final int colour;
  final List<String> members;
  const GraphGroup(this.name, this.colour, this.members);
}

/// How much bigger a group's bar is drawn than its canvas size. Zoomed out, a
/// box's name is too small to read, and the group names are then what say
/// where things are, so they hold their size on screen up to three times over.
double graphGroupScale(double zoom) => (1 / zoom).clamp(1.0, 3.0);

/// The file a group called [name] is saved as. The name is the file's name
/// too, less the letters Windows refuses.
String graphGroupFile(String dir, String name, String extension) {
  final stem = name.replaceAll(RegExp(r'[<>:"/\\|?*\x00-\x1f]'), '_');
  return '$dir/$stem.$extension';
}

/// What a row of a group's menu does.
enum _GroupDo { rename, colour, save, ungroup }

/// What a row of a note's menu does.
enum _NoteDo { edit, colour, delete }

/// How wide a note is drawn, in canvas units. Its height is its words'.
const double graphNoteWidth = 180;

/// The chip of the label palette a new note wears: the amber.
const int _noteColour = 2;

/// The group half of a graph canvas.
mixin GraphGroups<W extends StatefulWidget> on State<W> {
  /// The canvas's groups as the document has them.
  List<GraphGroup> get groups;

  /// Write the groups back as one undo step.
  void commitGroups(List<GraphGroup> next);

  /// Every box the canvas draws, by key, in canvas units.
  Map<String, Rect> get boxRects;

  List<String> get pickedKeys;

  /// Replace the pick. Called inside `setState`.
  void pickKeys(Iterable<String> keys);

  /// Whether a saved group can carry this box. The two ends of a graph stay
  /// behind, since every graph has its own.
  bool groupCarries(String key);

  /// The engine's text for these boxes saved as a group.
  String groupText(String name, int colour, List<String> members);

  /// What a saved group of this canvas is called on disk.
  String get groupExtension;

  /// Where saved groups go. A test names a folder of its own.
  String? get groupLibrary => presetsDirPath();

  /// The library's saved groups for this kind of graph.
  List<BridgePresetInfo> savedGroups();

  /// Put the saved group at [path] in the middle of the canvas.
  void insertSavedGroup(String path);

  /// The canvas's notes as the document has them.
  List<BridgeGraphNote> get notes;

  /// Write the notes back as one undo step.
  void commitNotes(List<BridgeGraphNote> next);

  /// A clear spot in the middle of the canvas, where a new note goes.
  Offset get noteSpot;

  /// Whether the toolbar's magnet is on, which a dragged note follows too.
  bool get groupSnaps;

  double get groupZoom;
  LumitUiState? get groupUi;

  /// The group whose bar was pressed last, while its boxes are still the pick.
  int? groupPick;

  /// The group whose name is being typed.
  int? groupRenaming;

  /// The group a dragged box would join if it were let go now.
  int? groupLit;

  /// The note whose words are being typed.
  int? noteEditing;

  /// Where a note that is not written yet will go. It joins the document
  /// when it has words, so a note abandoned empty never existed.
  Offset? _noteNew;

  /// A note being dragged, and where it has got to.
  (int, Offset)? _noteDrag;

  /// A press that landed in a name or a note being typed. The canvas takes
  /// the keyboard on every other press, which would end the typing.
  bool _typingPress = false;

  /// Whether the press being handled landed in a field of this mixin's.
  /// Asked once per press by the canvas, before it takes the keyboard.
  bool groupTypingPress() {
    final was = _typingPress;
    _typingPress = false;
    return was;
  }

  /// A bar's press, on its way to the canvas's own pointer handler.
  int? _pressed;
  final DoubleTap _taps = DoubleTap();
  int? _tapped;

  /// Whether group [a] sits inside group [b]. Two groups of the very same
  /// boxes nest in the order they were made.
  static bool _inside(List<GraphGroup> all, int a, int b) {
    if (a == b) return false;
    final outer = all[b].members.toSet();
    final inner = all[a].members.toSet();
    return outer.containsAll(inner) && (inner.length < outer.length || a < b);
  }

  /// The groups from the innermost out, so each comes after any inside it.
  static List<int> _innerFirst(List<GraphGroup> all) =>
      [for (var i = 0; i < all.length; i++) i]..sort((a, b) {
          final by = all[a].members.toSet().length -
              all[b].members.toSet().length;
          return by != 0 ? by : a - b;
        });

  /// The frame each group draws, or null where none of its boxes is on the
  /// canvas. An outer group's frame goes round the frames inside it.
  List<Rect?> groupRects() {
    final all = groups;
    final boxes = boxRects;
    final head = graphGroupHead * graphGroupScale(groupZoom);
    final rects = List<Rect?>.filled(all.length, null);
    final order = _innerFirst(all);
    for (final (n, g) in order.indexed) {
      rects[g] = graphGroupRect([
        for (final key in all[g].members)
          if (boxes[key] case final rect?) rect,
        for (final h in order.take(n))
          if (_inside(all, h, g))
            if (rects[h] case final rect?) rect,
      ], head: head);
    }
    return rects;
  }

  /// The groups with [keys] taken out of them. A group left empty goes.
  List<GraphGroup> groupsWithout(Iterable<String> keys) {
    final gone = keys.toSet();
    return [
      for (final g in groups)
        if (g.members.any((m) => !gone.contains(m)))
          GraphGroup(g.name, g.colour, [
            for (final m in g.members)
              if (!gone.contains(m)) m,
          ]),
    ];
  }

  /// The one group that holds every picked box and nothing else, if there is
  /// one: what Save group saves again without making another.
  int? get _groupOfPick {
    final picked = pickedKeys.toSet();
    final at = groups.indexWhere((g) =>
        g.members.length == picked.length && g.members.every(picked.contains));
    return at < 0 ? null : at;
  }

  bool get canGroup => pickedKeys.isNotEmpty;
  bool get canSaveGroup => pickedKeys.any(groupCarries);

  /// The groups with [boxes] framed as a new one, which comes last. A group
  /// round all of them goes on holding them, so the new one is inside it. A
  /// group wholly among them comes inside the new one. Any other lets go of
  /// the boxes it shares.
  List<GraphGroup> _grouped(List<String> boxes, String name) {
    final picked = boxes.toSet();
    final all = groups;
    return [
      for (final g in all)
        if (g.members.toSet().containsAll(picked) ||
            picked.containsAll(g.members))
          g
        else
          GraphGroup(g.name, g.colour, [
            for (final m in g.members)
              if (!picked.contains(m)) m,
          ]),
      // Chip 0 is the palette's grey, which reads as no colour at all.
      GraphGroup(name, all.length + 1, boxes),
    ];
  }

  /// Frame the picked boxes as a new group and start typing its name.
  void groupPicked() {
    final picked = pickedKeys;
    // Boxes that are a group already are not framed twice.
    if (picked.isEmpty || _groupOfPick != null) return;
    final next = _grouped(picked, l10n.graphGroupDefaultName);
    commitGroups(next);
    setState(() => groupPick = groupRenaming = next.length - 1);
  }

  /// With a group's bar pressed, take that frame away and leave its boxes.
  /// Otherwise take each picked box out of the innermost group it is in.
  void ungroupPicked() {
    final all = groups;
    if (groupPick case final at? when at < all.length) {
      commitGroups([...all]..removeAt(at));
      setState(() => groupPick = groupRenaming = null);
      return;
    }
    final order = _innerFirst(all);
    final leaves = <int, Set<String>>{};
    for (final key in pickedKeys) {
      final at = order.where((g) => all[g].members.contains(key)).firstOrNull;
      if (at != null) leaves.putIfAbsent(at, () => {}).add(key);
    }
    if (leaves.isEmpty) return;
    commitGroups([
      for (final (i, g) in all.indexed)
        if (leaves[i] == null)
          g
        // A group left with no box goes.
        else if (g.members.any((m) => !leaves[i]!.contains(m)))
          GraphGroup(g.name, g.colour, [
            for (final m in g.members)
              if (!leaves[i]!.contains(m)) m,
          ]),
    ]);
    setState(() => groupPick = groupRenaming = null);
  }

  void _set(int at, {String? name, int? colour}) {
    final all = groups;
    if (at >= all.length) return;
    commitGroups([
      for (final (i, g) in all.indexed)
        i == at
            ? GraphGroup(name ?? g.name, colour ?? g.colour, g.members)
            : g,
    ]);
  }

  /// The name typed on a bar. An empty one keeps the old name, since a group
  /// with no name has a bar with nothing on it to press.
  void renameGroup(int at, String name) {
    setState(() => groupRenaming = null);
    final trimmed = name.trim();
    if (trimmed.isEmpty || at >= groups.length || trimmed == groups[at].name) {
      return;
    }
    _set(at, name: trimmed);
  }

  /// Save the picked boxes to the library as a group, asking its name. Boxes
  /// that are not a group yet become one under that name, so what is on the
  /// canvas and what is in the library are called the same thing.
  Future<void> saveGroup([int? at]) async {
    at ??= groupPick ?? _groupOfPick;
    if (at != null && at >= groups.length) at = null;
    final members = at == null ? pickedKeys : groups[at].members;
    if (!members.any(groupCarries)) return;
    final name = await askThemeName(
      context,
      title: l10n.graphSaveGroupTitle,
      suggested: at == null ? l10n.graphGroupDefaultName : groups[at].name,
    );
    final dir = groupLibrary;
    if (name == null || dir == null || !mounted) return;
    final file = File(graphGroupFile(dir, name, groupExtension));
    if (file.existsSync() &&
        !await askBefore(
            context,
            l10n.graphSaveGroupReplace(name),
            l10n.graphSaveGroupReplaceYes)) {
      return;
    }
    if (!mounted) return;

    final int colour;
    if (at == null) {
      final next = _grouped(members, name);
      colour = next.last.colour;
      commitGroups(next);
    } else {
      colour = groups[at].colour;
      if (name != groups[at].name) _set(at, name: name);
    }
    try {
      file.writeAsStringSync(groupText(name, colour, members));
    } catch (_) {
      // The library is the user's own folder. A write it refused leaves the
      // group on the canvas, which is the half that did land.
    }
  }

  /// The saved groups' window. A row picked there is inserted here.
  Future<void> _openSaved() async {
    final path = await showLumitModal<String>(
      context: context,
      builder: (close) => _SavedGroups(
        lister: savedGroups,
        folder: groupLibrary,
        extension: groupExtension,
        close: close,
      ),
    );
    if (path != null && mounted) insertSavedGroup(path);
  }

  Future<void> _menu(int at, Offset position) async {
    final picked = await showMenuAt<_GroupDo>(
      context: context,
      position: position,
      width: 180,
      rows: (close) => [
        MenuRow(
          key: const ValueKey('graph-group-menu-rename'),
          onPressed: () => close(_GroupDo.rename),
          child: Text(l10n.rename),
        ),
        MenuRow(
          key: const ValueKey('graph-group-menu-colour'),
          onPressed: () => close(_GroupDo.colour),
          child: Text(l10n.graphGroupColour),
        ),
        MenuRow(
          key: const ValueKey('graph-group-menu-save'),
          onPressed: () => close(_GroupDo.save),
          child: Text(l10n.graphSaveGroup),
        ),
        MenuRow(
          key: const ValueKey('graph-group-menu-ungroup'),
          onPressed: () => close(_GroupDo.ungroup),
          child: Text(l10n.graphUngroup),
        ),
      ],
    );
    if (picked == null || !mounted || at >= groups.length) return;
    switch (picked) {
      case _GroupDo.rename:
        setState(() => groupRenaming = at);
      case _GroupDo.colour:
        final colour =
            await showLabelPicker(context, position, keyPrefix: 'graph-group');
        if (colour != null && mounted) _set(at, colour: colour);
      case _GroupDo.save:
        await saveGroup(at);
      case _GroupDo.ungroup:
        commitGroups([...groups]..removeAt(at));
        setState(() => groupPick = null);
    }
  }

  /// A press on a group's bar, read by the canvas's own pointer handler since
  /// that is where a drag of boxes is kept. True when the press was a bar's:
  /// a plain one picks the group's boxes and hands back the drag that moves
  /// them, a second one renames it and the right button opens its menu.
  /// [unless] is for a press that was something else's first.
  bool groupDown(
    PointerDownEvent event,
    Offset at,
    ValueChanged<GraphNodeDrag> drag, {
    bool unless = false,
  }) {
    final g = _pressed;
    _pressed = null;
    if (unless || g == null || g >= groups.length) return false;
    // The middle button pans, over a bar as anywhere else.
    if (event.buttons == kMiddleMouseButton) return false;
    if (event.buttons == kSecondaryMouseButton) {
      _menu(g, event.position);
      return true;
    }
    final again = _taps.tap(at: event.localPosition, slop: 6) && _tapped == g;
    _tapped = g;
    if (again) {
      setState(() => groupRenaming = g);
      return true;
    }
    final keys = HardwareKeyboard.instance;
    final adds = keys.isShiftPressed ||
        keys.isControlPressed ||
        keys.isMetaPressed;
    final boxes = boxRects;
    final members = [
      for (final key in groups[g].members)
        if (boxes.containsKey(key)) key,
    ];
    if (members.isEmpty) return true;
    setState(() {
      pickKeys({if (adds) ...pickedKeys, ...members});
      groupPick = g;
      drag(GraphNodeDrag(
        members.first,
        at,
        {for (final key in members) key: boxes[key]!.topLeft},
      ));
    });
    return true;
  }

  /// The group the dragged boxes would move into with the pointer at [at]:
  /// the innermost frame under it. A frame already round all of them is not
  /// one, and nor is the frame of a group being dragged whole.
  int? _dropInto(Set<String> dragged, Offset at) {
    final all = groups;
    if (all.isEmpty || dragged.isEmpty) return null;
    final rects = groupRects();
    for (final g in _innerFirst(all)) {
      if (!(rects[g]?.contains(at) ?? false)) continue;
      final members = all[g].members.toSet();
      if (members.containsAll(dragged) || dragged.containsAll(members)) {
        continue;
      }
      return g;
    }
    return null;
  }

  /// Light the frame the boxes being dragged would move into. Called as they
  /// move, with the pointer in canvas units.
  void groupDragOver(Iterable<String> dragged, Offset at) =>
      groupLit = _dropInto(dragged.toSet(), at);

  /// The groups once the boxes just let go have moved into the frame the
  /// pointer was over, or null when it was over none. That group and the ones
  /// round it take them in, a group dragged whole comes along as it is, and
  /// any other lets go of them.
  List<GraphGroup>? groupsAfterDrop(Iterable<String> dragged, Offset at) {
    groupLit = null;
    final moved = dragged.toSet();
    final into = _dropInto(moved, at);
    if (into == null) return null;
    final all = groups;
    return [
      for (final (i, g) in all.indexed)
        if (i == into || _inside(all, into, i))
          GraphGroup(g.name, g.colour, {...g.members, ...moved}.toList())
        else if (moved.containsAll(g.members))
          g
        else
          GraphGroup(g.name, g.colour, [
            for (final m in g.members)
              if (!moved.contains(m)) m,
          ]),
    ];
  }

  /// Start a new note in the middle of the canvas.
  void addNote() => setState(() {
        noteEditing = null;
        _noteNew = noteSpot;
      });

  BridgeGraphNote _note(BridgeGraphNote from,
          {String? text, int? colour, Offset? at}) =>
      BridgeGraphNote(
        text: text ?? from.text,
        colour: colour ?? from.colour,
        x: at?.dx ?? from.x,
        y: at?.dy ?? from.y,
      );

  /// The words typed into a note. One left with none is taken away, and a
  /// new one with none is never made.
  void _noteDone(int? at, String typed) {
    final text = typed.trim();
    final fresh = _noteNew;
    setState(() => noteEditing = _noteNew = null);
    final all = notes;
    if (at == null) {
      if (fresh == null || text.isEmpty) return;
      commitNotes([
        ...all,
        BridgeGraphNote(
            text: text, colour: _noteColour, x: fresh.dx, y: fresh.dy),
      ]);
    } else if (at < all.length && text != all[at].text) {
      commitNotes([
        for (final (i, n) in all.indexed)
          if (i != at) n else if (text.isNotEmpty) _note(n, text: text),
      ]);
    }
  }

  Future<void> _noteMenu(int at, Offset position) async {
    final picked = await showMenuAt<_NoteDo>(
      context: context,
      position: position,
      width: 160,
      rows: (close) => [
        MenuRow(
          key: const ValueKey('graph-note-menu-edit'),
          onPressed: () => close(_NoteDo.edit),
          child: Text(l10n.graphNoteEdit),
        ),
        MenuRow(
          key: const ValueKey('graph-note-menu-colour'),
          onPressed: () => close(_NoteDo.colour),
          child: Text(l10n.graphGroupColour),
        ),
        MenuRow(
          key: const ValueKey('graph-note-menu-delete'),
          onPressed: () => close(_NoteDo.delete),
          child: Text(l10n.delete),
        ),
      ],
    );
    if (picked == null || !mounted || at >= notes.length) return;
    switch (picked) {
      case _NoteDo.edit:
        setState(() => noteEditing = at);
      case _NoteDo.colour:
        final colour =
            await showLabelPicker(context, position, keyPrefix: 'graph-note');
        if (colour == null || !mounted || at >= notes.length) return;
        commitNotes([
          for (final (i, n) in notes.indexed)
            i == at ? _note(n, colour: colour) : n,
        ]);
      case _NoteDo.delete:
        commitNotes([...notes]..removeAt(at));
    }
  }

  /// A dragged note let go: on the dot grid with the magnet on, and written
  /// back once.
  void _noteDropped() {
    final drag = _noteDrag;
    if (drag == null) return;
    setState(() => _noteDrag = null);
    final (at, where) = drag;
    if (at >= notes.length) return;
    final to = groupSnaps
        ? Offset(
            (where.dx / graphDotGrid).round() * graphDotGrid,
            (where.dy / graphDotGrid).round() * graphDotGrid,
          )
        : where;
    commitNotes([
      for (final (i, n) in notes.indexed) i == at ? _note(n, at: to) : n,
    ]);
  }

  /// The notes, for the canvas's stack: over the groups' frames and under
  /// the boxes. [onOwnPress] is what a box's own controls call.
  List<Widget> graphNotes(LumitTheme t, VoidCallback onOwnPress) {
    void typing() {
      _typingPress = true;
      onOwnPress();
    }

    // A note being dragged is drawn where the drag has got it to.
    Offset place(int i, BridgeGraphNote note) => switch (_noteDrag) {
          (final at, final where) when at == i => where,
          _ => Offset(note.x, note.y),
        };

    return [
      for (final (i, note) in notes.indexed)
        Positioned(
          key: ValueKey<String>('graph-note-$i'),
          left: place(i, note).dx,
          top: place(i, note).dy,
          width: graphNoteWidth,
          child: GraphNoteCard(
            text: note.text,
            colour: t.labelColour(note.colour),
            editing: noteEditing == i,
            onOwnPress: noteEditing == i ? typing : onOwnPress,
            onDone: (text) => _noteDone(i, text),
            onCancel: () => setState(() => noteEditing = null),
            onEdit: () => setState(() {
              _noteNew = null;
              noteEditing = i;
            }),
            onMenu: (position) => _noteMenu(i, position),
            onDrag: (delta) =>
                setState(() => _noteDrag = (i, place(i, note) + delta)),
            onDragEnd: _noteDropped,
          ),
        ),
      if (_noteNew case final at?)
        Positioned(
          key: const ValueKey('graph-note-new'),
          left: at.dx,
          top: at.dy,
          width: graphNoteWidth,
          child: GraphNoteCard(
            text: '',
            colour: t.labelColour(_noteColour),
            editing: true,
            onOwnPress: typing,
            onDone: (text) => _noteDone(null, text),
            onCancel: () => setState(() => _noteNew = null),
          ),
        ),
    ];
  }

  /// The canvas's group keys. They follow the Timeline's own chords for
  /// grouping layers, so one rebinding moves both.
  KeyEventResult groupKey(KeyEvent event) {
    final action =
        groupUi?.keymap.actionFor(BridgeKeyContext.timeline, event);
    if (action == 'layer.group') {
      groupPicked();
      return KeyEventResult.handled;
    }
    if (action == 'layer.ungroup') {
      ungroupPicked();
      return KeyEventResult.handled;
    }
    if (groupPick case final g?
        when event.logicalKey == LogicalKeyboardKey.f2 ||
            event.logicalKey == LogicalKeyboardKey.enter) {
      setState(() => groupRenaming = g);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  /// The frames, for the bottom of the canvas's stack of boxes. [onOwnPress]
  /// is what a box's own controls call, so a click in the name being typed is
  /// not read as a press on the canvas.
  List<Widget> groupWashes(LumitTheme t, VoidCallback onOwnPress) {
    final rects = groupRects();
    final scale = graphGroupScale(groupZoom);
    final all = groups;
    return [
      // Outermost first, so a group inside another is drawn over it.
      for (final i in _innerFirst(all).reversed)
        if ((all[i], rects[i]) case (final group, final rect?))
          Positioned.fromRect(
            key: ValueKey<String>('graph-group-$i'),
            rect: rect,
            child: GraphGroupWash(
              name: group.name,
              colour: t.labelColour(group.colour),
              scale: scale,
              picked: groupPick == i,
              lit: groupLit == i,
              renaming: groupRenaming == i,
              onPress: () => _pressed = i,
              onOwnPress: () {
                _typingPress = true;
                onOwnPress();
              },
              onRenamed: (name) => renameGroup(i, name),
              onRenameCancelled: () => setState(() => groupRenaming = null),
            ),
          ),
    ];
  }

  /// The toolbar's group and note buttons, shared so both canvases word them
  /// alike.
  List<Widget> groupButtons(LumitTheme t) => [
        LumitTooltip(
          message: l10n.tipGraphGroup,
          action: 'layer.group',
          child: HouseButton(
            key: const ValueKey('graph-group'),
            small: true,
            frameless: true,
            padding: const EdgeInsets.symmetric(horizontal: 4),
            onPressed: canGroup ? groupPicked : null,
            child: glyph.LumitIcon(LumitIcons.group,
                size: graphIconSize,
                colour: canGroup ? t.textPrimary : t.textDisabled),
          ),
        ),
        const SizedBox(width: 6),
        LumitTooltip(
          message: l10n.tipGraphNote,
          child: HouseButton(
            key: const ValueKey('graph-note'),
            small: true,
            frameless: true,
            padding: const EdgeInsets.symmetric(horizontal: 4),
            onPressed: addNote,
            child: glyph.LumitIcon(LumitIcons.note,
                size: graphIconSize, colour: t.textMuted),
          ),
        ),
        const SizedBox(width: 6),
        HouseButton(
          key: const ValueKey('graph-save-group'),
          small: true,
          frameless: true,
          onPressed: canSaveGroup ? saveGroup : null,
          child: Text(l10n.graphSaveGroup, style: t.small),
        ),
        const SizedBox(width: 6),
        LumitTooltip(
          message: l10n.graphSavedGroups,
          child: HouseButton(
            key: const ValueKey('graph-saved-groups'),
            small: true,
            frameless: true,
            padding: const EdgeInsets.symmetric(horizontal: 4),
            onPressed: _openSaved,
            child: glyph.LumitIcon(LumitIcons.folder,
                size: graphIconSize, colour: t.textMuted),
          ),
        ),
      ];
}

/// One group's frame: a tinted wash with the group's name on a bar along its
/// top. The colour is a chip of the label palette. The canvas's own colours
/// are the port types and nothing else, so a group borrows the other palette.
class GraphGroupWash extends StatelessWidget {
  final String name;
  final Color colour;

  /// How much bigger than its canvas size the bar is drawn ([graphGroupScale]).
  final double scale;
  final bool picked;
  final bool lit;
  final bool renaming;
  final VoidCallback onPress;
  final VoidCallback onOwnPress;
  final ValueChanged<String> onRenamed;
  final VoidCallback onRenameCancelled;

  const GraphGroupWash({
    super.key,
    required this.name,
    required this.colour,
    required this.scale,
    required this.picked,
    required this.lit,
    required this.renaming,
    required this.onPress,
    required this.onOwnPress,
    required this.onRenamed,
    required this.onRenameCancelled,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final corner = Radius.circular(t.tokens.cardRadius);
    final head = graphGroupHead * scale;
    return Stack(
      children: [
        // The wash takes no pointer: a press inside a frame is a press on the
        // canvas, which is what lets a selection box start there.
        Positioned.fill(
          child: IgnorePointer(
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: colour.withValues(alpha: lit ? 0.14 : 0.07),
                border: Border.all(
                  color: colour.withValues(alpha: picked || lit ? 0.9 : 0.35),
                ),
                borderRadius: BorderRadius.all(corner),
              ),
            ),
          ),
        ),
        Positioned(
          left: 0,
          top: 0,
          right: 0,
          height: head,
          child: renaming
              ? Listener(
                  onPointerDown: (_) => onOwnPress(),
                  child: GraphNameField(
                    key: const ValueKey('graph-group-rename'),
                    initial: name,
                    onDone: onRenamed,
                    onCancel: onRenameCancelled,
                  ),
                )
              : HoverMark(
                  cursor: SystemMouseCursors.grab,
                  builder: (context, hover) => Listener(
                    onPointerDown: (_) => onPress(),
                    behavior: HitTestBehavior.opaque,
                    child: Container(
                      decoration: BoxDecoration(
                        color: colour.withValues(
                            alpha: picked ? 0.42 : 0.2 + 0.12 * hover),
                        borderRadius: BorderRadius.vertical(top: corner),
                      ),
                      padding: EdgeInsets.symmetric(horizontal: 8 * scale),
                      alignment: Alignment.centerLeft,
                      child: Text(
                        name,
                        style: t.kickerOn.copyWith(
                          fontSize: t.kickerOn.fontSize! * scale,
                          color: Color.lerp(colour, t.textPrimary,
                              picked ? 1 : 0.55 + 0.45 * hover),
                        ),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                  ),
                ),
        ),
      ],
    );
  }
}

/// One note: a tinted card with its words on it, or the field they are typed
/// in. A note with no [onEdit] is one still being written for the first time.
class GraphNoteCard extends StatelessWidget {
  final String text;
  final Color colour;
  final bool editing;
  final VoidCallback onOwnPress;
  final ValueChanged<String> onDone;
  final VoidCallback onCancel;
  final VoidCallback? onEdit;
  final ValueChanged<Offset>? onMenu;
  final ValueChanged<Offset>? onDrag;
  final VoidCallback? onDragEnd;

  const GraphNoteCard({
    super.key,
    required this.text,
    required this.colour,
    required this.editing,
    required this.onOwnPress,
    required this.onDone,
    required this.onCancel,
    this.onEdit,
    this.onMenu,
    this.onDrag,
    this.onDragEnd,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final card = BoxDecoration(
      color: Color.alphaBlend(colour.withValues(alpha: 0.18), t.surface1),
      border: Border(left: BorderSide(color: colour, width: 2)),
      borderRadius: BorderRadius.circular(t.tokens.cardRadius),
    );
    if (editing) {
      return Listener(
        onPointerDown: (_) => onOwnPress(),
        child: Container(
          height: 96,
          decoration: card,
          padding: const EdgeInsets.only(left: 2),
          child: GraphNameField(
            key: const ValueKey('graph-note-field'),
            initial: text,
            multiline: true,
            hint: l10n.graphNoteHint,
            onDone: onDone,
            onCancel: onCancel,
          ),
        ),
      );
    }
    return Listener(
      onPointerDown: (event) {
        onOwnPress();
        if (event.buttons == kSecondaryMouseButton) onMenu?.call(event.position);
      },
      child: MouseRegion(
        cursor: SystemMouseCursors.grab,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onDoubleTap: onEdit,
          onPanUpdate: (details) => onDrag?.call(details.delta),
          onPanEnd: (_) => onDragEnd?.call(),
          child: Container(
            decoration: card,
            padding: const EdgeInsets.fromLTRB(10, 6, 8, 7),
            child: Text(text,
                style: t.body.copyWith(color: t.textPrimary, height: 1.35)),
          ),
        ),
      ),
    );
  }
}

/// The groups saved to the library for one kind of graph: click one to insert
/// it, or rename or delete it here.
class _SavedGroups extends StatefulWidget {
  final List<BridgePresetInfo> Function() lister;
  final String? folder;
  final String extension;

  /// Shuts the window, with the path of the group to insert or with nothing.
  final void Function(String?) close;

  const _SavedGroups({
    required this.lister,
    required this.folder,
    required this.extension,
    required this.close,
  });

  @override
  State<_SavedGroups> createState() => _SavedGroupsState();
}

class _SavedGroupsState extends State<_SavedGroups> {
  /// Each saved group with its colour chip and how many boxes it holds, read
  /// off its file when the list is made and not as it is drawn.
  late List<(BridgePresetInfo, int, int)> _rows = _read();

  List<(BridgePresetInfo, int, int)> _read() => [
        for (final info in widget.lister())
          switch (_facts(info.path)) {
            (final colour, final boxes) => (info, colour, boxes),
          },
      ];

  (int, int) _facts(String path) {
    try {
      final saved = jsonDecode(File(path).readAsStringSync()) as Map;
      return (
        (saved['colour'] as num?)?.toInt() ?? 0,
        ((saved['nodes'] as List?)?.length ?? 0) +
            ((saved['effects'] as List?)?.length ?? 0),
      );
    } catch (_) {
      // Not a file this build can read. It is listed all the same.
      return (0, 0);
    }
  }

  Future<void> _rename(BridgePresetInfo saved) async {
    final name = await askThemeName(
      context,
      title: l10n.graphRenameSavedGroup,
      suggested: saved.name,
      confirm: l10n.rename,
    );
    if (name == null || name == saved.name) return;
    try {
      renameSavedGroup(
        path: saved.path,
        name: name,
        to: graphGroupFile(File(saved.path).parent.path, name, widget.extension),
      );
    } catch (_) {
      // The name is another saved group's, so this one keeps its own.
    }
    if (mounted) setState(() => _rows = _read());
  }

  Future<void> _delete(BridgePresetInfo saved) async {
    if (!await askBefore(
        context, l10n.graphDeleteSavedGroup(saved.name), l10n.delete)) {
      return;
    }
    try {
      File(saved.path).deleteSync();
    } catch (_) {
      // Gone already, or not ours to delete. The list says which.
    }
    if (mounted) setState(() => _rows = _read());
  }

  Widget _mark(LumitTheme t, String key, String icon, String tip,
          VoidCallback onTap) =>
      LumitTooltip(
        message: tip,
        child: HoverMark(
          swells: true,
          builder: (context, lit) => GestureDetector(
            key: ValueKey<String>(key),
            behavior: HitTestBehavior.opaque,
            onTap: onTap,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
              child: glyph.LumitIcon(icon,
                  size: graphIconSize,
                  colour: Color.lerp(t.textMuted, t.textPrimary, lit)),
            ),
          ),
        ),
      );

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      width: 340,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(10, 4, 10, 8),
            child: Text(l10n.graphSavedGroups, style: t.bodyPrimary),
          ),
          if (_rows.isEmpty)
            Padding(
              padding: const EdgeInsets.fromLTRB(10, 4, 10, 8),
              child: Text(l10n.graphSavedGroupsEmpty, style: t.small),
            )
          else
            ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 320),
              child: SingleChildScrollView(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    for (final (saved, colour, boxes) in _rows)
                      MenuRow(
                        key: ValueKey<String>('graph-saved-${saved.name}'),
                        onPressed: () => widget.close(saved.path),
                        child: Row(
                          children: [
                            Container(
                              width: 8,
                              height: 8,
                              color: t.labelColour(colour),
                            ),
                            const SizedBox(width: 8),
                            Expanded(
                              child: Text(saved.name,
                                  style: t.bodyPrimary,
                                  overflow: TextOverflow.ellipsis),
                            ),
                            Text(l10n.graphSavedGroupBoxes(boxes),
                                style: t.small),
                            const SizedBox(width: 6),
                            _mark(t, 'graph-saved-rename-${saved.name}',
                                LumitIcons.rename, l10n.rename,
                                () => _rename(saved)),
                            _mark(t, 'graph-saved-delete-${saved.name}',
                                LumitIcons.delete, l10n.delete,
                                () => _delete(saved)),
                          ],
                        ),
                      ),
                  ],
                ),
              ),
            ),
          const SizedBox(height: 8),
          Row(
            children: [
              HouseButton(
                key: const ValueKey('graph-saved-folder'),
                small: true,
                frameless: true,
                onPressed: widget.folder == null
                    ? null
                    : () => revealInFolder(path: widget.folder!),
                child: Text(l10n.graphSavedGroupsFolder, style: t.small),
              ),
              const Spacer(),
              HouseButton(
                small: true,
                onPressed: () => widget.close(null),
                child: Text(l10n.close),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
