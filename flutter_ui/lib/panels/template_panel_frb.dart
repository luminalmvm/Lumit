// The Graphics template panel: the fronted composition's controls, listed as
// a copy of it will show them, to set, rename, group, reorder and remove.
//
// The template is read when the document moves, by the keeper the rest of
// the app shares, and never while drawing. Every edit is one call to the
// engine.

import 'dart:async';

import 'package:flutter/gestures.dart'
    show kDoubleTapSlop, kDoubleTapTimeout, kPrimaryButton;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/template.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart' show UuidValue;

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show MenuEntry;
import '../shell/template_windows.dart';
import '../state/dock.dart'
    show Panel, PanelPane, activatePanelTab, setPanelVisible;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import 'captions_states_frb.dart' show captionsMark;
import 'captions_tools_frb.dart' show popupBelow;
import 'fx_section.dart';
import 'keyframe_controls_frb.dart' show stopwatchGlyph;
import 'placeholder.dart';
import 'template_foot.dart';
import 'template_reveal.dart';
import 'template_rows_frb.dart';
import 'timeline_layer_menu_frb.dart' show showMenuEntriesAt;

/// Controls being dragged to another place in the list, in the template's
/// order.
class TemplateControlsDrag {
  final List<String> ids;
  const TemplateControlsDrag(this.ids);
}

/// How far a control's further rows and a menu's options sit in from it.
const double _indent = 14;

class TemplatePanelFrb extends StatefulWidget {
  const TemplatePanelFrb({super.key});

  @override
  State<TemplatePanelFrb> createState() => _TemplatePanelFrbState();
}

class _TemplatePanelFrbState extends State<TemplatePanelFrb> {
  late final LumitState _app = Provider.of<LumitState>(context, listen: false);
  late final LumitUiState _ui =
      Provider.of<LumitUiState>(context, listen: false);

  /// The fronted composition, its name, and whether it is a node graph.
  CompositionReference? _comp;
  String _compName = '';
  bool _graph = false;

  /// The composition and the revision the name and the text rows were read
  /// at.
  UuidValue? _readComp;
  int? _readRevision;

  /// The template as last read, and the list drawn from it.
  BridgeTemplate? _template;
  List<TemplateLine> _lines = const [];

  /// The menus whose options are the person's own: rows of a Custom controls
  /// effect.
  Set<String> _ownMenus = const {};

  /// The Text layer behind each text control that shows further rows, by the
  /// control's id.
  Map<String, BridgeTextDocument> _texts = const {};

  /// The controls picked, and the one the last plain or Ctrl click landed on,
  /// for Shift to run from.
  final Set<String> _picked = {};
  String? _anchor;

  /// The end of the pick that the arrows move.
  String? _lead;

  /// A picked row pressed with no modifier: the pick narrows to it on
  /// release, unless the press became a drag of all of them.
  String? _narrowTo;

  /// Groups folded shut, and menus unfolded to their options.
  final Set<String> _shutGroups = {};
  final Set<String> _openMenus = {};

  /// What is being renamed in place: a control, a group, or a menu's option.
  String? _renaming;
  String? _renamingGroup;
  (String, int)? _renamingOption;

  /// Where a dragged set would land: the line it is over, and which edge.
  final ValueNotifier<({String line, bool below})?> _dropAt =
      ValueNotifier(null);

  final ScrollController _scroll = ScrollController();
  final Map<String, GlobalKey> _rowKeys = {};

  bool Function()? _priorDelete;
  VoidCallback? _escapeRelease;

  @override
  void initState() {
    super.initState();
    _ui.model.addListener(_sync);
    _ui.templateControls.addListener(_sync);
    _priorDelete = _ui.deleteClaim;
    _ui.deleteClaim = _deleteClaim;
    HardwareKeyboard.instance.addHandler(_onKey);
    _escapeRelease = EscapeLadder.register(EscapeRung.gesture, _escape);
    _sync();
  }

  @override
  void dispose() {
    _escapeRelease?.call();
    HardwareKeyboard.instance.removeHandler(_onKey);
    if (_ui.deleteClaim == _deleteClaim) _ui.deleteClaim = _priorDelete;
    _ui.model.removeListener(_sync);
    _ui.templateControls.removeListener(_sync);
    _dropAt.dispose();
    _scroll.dispose();
    super.dispose();
  }

  // --- Reading ----------------------------------------------------------------

  /// Take the template the keeper holds for the fronted composition, and
  /// read what the list needs beside it when the document has moved.
  void _sync() {
    if (!mounted) return;
    final comp = _ui.selectedComp;
    final held = _ui.templateControls;
    final revision = _ui.model.heldRevision;
    final template = comp != null && held.comp?.internalid == comp.internalid
        ? held.template
        : null;
    final moved =
        comp?.internalid != _readComp || revision != _readRevision;
    var name = _compName;
    var texts = _texts;
    var ownMenus = _ownMenus;
    if (moved) {
      _readComp = comp?.internalid;
      _readRevision = revision;
      name = '';
      try {
        name = comp?.getSettings().name ?? '';
      } catch (_) {
        // The composition has gone. The next read shows none.
      }
    }
    if (moved || !identical(template, _template)) {
      final layers = template == null
          ? const <BridgeLayerEntry>[]
          : _ui.model.layers;
      final own = <String>{};
      final read = <String, BridgeTextDocument>{};
      for (final control in template?.controls ?? const <BridgeControl>[]) {
        if (control.gone) continue;
        final entry = layers
            .where((e) => e.layer.internallayerId == control.row.layer)
            .firstOrNull;
        if (entry == null) continue;
        if (control.kind == BridgeControlKind.menu &&
            control.row.kind == BridgeRowKind.param &&
            entry.info.effects.any((fx) =>
                fx.id == control.row.owner && fx.name == 'custom_controls')) {
          own.add(control.id);
        }
        if (control.kind == BridgeControlKind.text &&
            (control.font || control.size || control.colour)) {
          try {
            if (entry.layer.getText() case final text?) {
              read[control.id] = text;
            }
          } catch (_) {
            // The layer has gone. Its rows show a dash.
          }
        }
      }
      ownMenus = own;
      texts = read;
    }
    setState(() {
      _comp = comp;
      _compName = name;
      _graph = _ui.model.heldIsNodeGraph;
      _template = template;
      _ownMenus = ownMenus;
      _texts = texts;
      final ids = {
        for (final c in template?.controls ?? const <BridgeControl>[]) c.id
      };
      _picked.retainAll(ids);
      _openMenus.retainAll(ids);
      _rowKeys.removeWhere((id, _) => !ids.contains(id));
      if (_anchor != null && !ids.contains(_anchor)) _anchor = null;
      if (_lead != null && !ids.contains(_lead)) _lead = null;
      if (_renaming != null && !ids.contains(_renaming)) _renaming = null;
      if (_renamingOption != null && !ids.contains(_renamingOption!.$1)) {
        _renamingOption = null;
      }
      _relist();
    });
  }

  /// Lay the list out again from the template and the folds.
  void _relist() {
    final template = _template;
    _lines = template == null
        ? const []
        : templateLines(
            template.controls,
            shutGroups: _shutGroups,
            // The menu being recorded shows its options while it is.
            openMenus: {
              ..._openMenus,
              if (template.recording case final recording?) recording.menu,
            },
            ownMenus: _ownMenus,
          );
  }

  List<BridgeControl> get _controls => _template?.controls ?? const [];

  /// The controls on screen, in the list's order.
  List<BridgeControl> get _shown => [
        for (final line in _lines)
          if (line is TemplateControlLine) line.control,
      ];

  // --- Editing ----------------------------------------------------------------

  /// Run [edit], read it back, and say in the status line when the engine
  /// turns it down.
  bool _try(void Function() edit, {String? refused}) {
    try {
      edit();
    } catch (_) {
      _app.postNotice(refused ?? l10n.templateEditFailed, error: true);
      return false;
    }
    _app.notifyDocumentChanged();
    _ui.model.refresh();
    return true;
  }

  void _setValue(BridgeControl control, BridgeControlValue value) {
    final comp = _comp;
    if (comp == null) return;
    _try(() => comp.setControlValue(id: control.id, value: value));
  }

  void _renameControl(BridgeControl control, String text) {
    final comp = _comp;
    final label = text.trim();
    setState(() => _renaming = null);
    if (comp == null || label.isEmpty || label == control.label) return;
    _try(() => comp.renameControl(id: control.id, label: label));
  }

  void _renameGroup(String group, String text) {
    final comp = _comp;
    final name = text.trim();
    setState(() => _renamingGroup = null);
    if (comp == null || name.isEmpty || name == group) return;
    if (_try(() => comp.renameControlGroup(group: group, name: name)) &&
        _shutGroups.remove(group)) {
      setState(() {
        _shutGroups.add(name);
        _relist();
      });
    }
  }

  void _remove(Iterable<String> ids) {
    final comp = _comp;
    final gone = [
      for (final c in _controls)
        if (ids.contains(c.id)) c.id,
    ];
    if (comp == null || gone.isEmpty) return;
    _try(() => comp.removeControls(ids: gone));
  }

  bool _deleteClaim() {
    if (!mounted || _ui.activePanel != Panel.graphicsTemplate) {
      return _priorDelete?.call() ?? false;
    }
    // Taken with nothing picked too: Delete in this panel is about its
    // controls, and must not fall through to the selected layers.
    if (_picked.isNotEmpty) _remove(_picked.toList());
    return true;
  }

  /// Move [ids] to before [before], or last, and into [group].
  void _moveTo(Iterable<String> ids, {String? before, String? group}) {
    final comp = _comp;
    final moved = [
      for (final c in _controls)
        if (ids.contains(c.id)) c.id,
    ];
    if (comp == null || moved.isEmpty) return;
    _try(() => comp.moveControls(ids: moved, before: before, group: group));
  }

  /// A group nobody has named yet: the word, then the word and a number.
  String _freshGroup() {
    final taken = templateGroups(_controls);
    final base = l10n.templateGroup;
    if (!taken.contains(base)) return base;
    for (var n = 2;; n++) {
      if (!taken.contains('$base $n')) return '$base $n';
    }
  }

  /// Put [ids] in a new group and open its name for typing.
  void _newGroup(Iterable<String> ids) {
    final name = _freshGroup();
    _moveTo(ids, group: name);
    if (templateGroups(_controls).contains(name)) {
      setState(() {
        _shutGroups.remove(name);
        _renamingGroup = name;
        _relist();
      });
    }
  }

  // --- Dropping ---------------------------------------------------------------

  /// The first of [from] that is not among [moved].
  String? _firstStill(Iterable<BridgeControl> from, Set<String> moved) =>
      from.where((c) => !moved.contains(c.id)).firstOrNull?.id;

  /// What follows the last control of [group] in the template's order, so a
  /// control put before it ends the group.
  String? _afterGroup(String group, Set<String> moved) {
    final last = _controls.lastIndexWhere((c) => c.group == group);
    return last < 0 ? null : _firstStill(_controls.skip(last + 1), moved);
  }

  /// A drop on [control]: above it, or below it, and into its group.
  void _dropOnControl(List<String> ids, BridgeControl control, bool below) {
    final moved = ids.toSet();
    final at = _controls.indexWhere((c) => c.id == control.id);
    if (at < 0) return;
    _moveTo(
      ids,
      before: below ? _firstStill(_controls.skip(at + 1), moved) : control.id,
      group: control.group,
    );
  }

  /// A drop on a group's heading: first in the group.
  void _dropOnGroup(List<String> ids, String group) {
    final moved = ids.toSet();
    _moveTo(
      ids,
      before: _firstStill(_controls.where((c) => c.group == group), moved) ??
          _afterGroup(group, moved),
      group: group,
    );
  }

  /// A drop above everything: first, and in no group.
  void _dropOnTop(List<String> ids) =>
      _moveTo(ids, before: _firstStill(_controls, ids.toSet()));

  /// A drop under everything: after the last line there is.
  void _dropOnEnd(List<String> ids) {
    final last = _lines.lastWhere(
        (l) => l is TemplateControlLine || l is TemplateGroupLine,
        orElse: () => const TemplateGroupLine('', open: true));
    switch (last) {
      case TemplateControlLine(:final control):
        _dropOnControl(ids, control, true);
      case TemplateGroupLine(:final name) when name.isNotEmpty:
        _moveTo(ids, before: _afterGroup(name, ids.toSet()), group: name);
      default:
        break;
    }
  }

  /// Whether a drag at [global] is over the lower half of [control]'s row.
  bool _below(String control, Offset global) {
    final box = _rowKeys[control]?.currentContext?.findRenderObject();
    if (box is! RenderBox || !box.hasSize) return false;
    return box.globalToLocal(global).dy > box.size.height / 2;
  }

  /// A line that takes a dragged set of controls, with the drop line on the
  /// edge they would land against.
  Widget _target(
    LumitTheme t, {
    required String line,
    required void Function(List<String> ids, bool below) onDrop,
    bool Function(List<String> ids)? accepts,
    bool Function(Offset global)? below,
    required Widget child,
  }) {
    final edge = BorderSide(color: t.accent, width: fxDropLineWidth);
    return DragTarget<TemplateControlsDrag>(
      onWillAcceptWithDetails: (d) => accepts?.call(d.data.ids) ?? true,
      // A line that would turn the drop down hears the move too, and shows
      // nothing.
      onMove: (d) {
        if (accepts?.call(d.data.ids) ?? true) {
          _dropAt.value = (line: line, below: below?.call(d.offset) ?? true);
        } else if (_dropAt.value?.line == line) {
          _dropAt.value = null;
        }
      },
      onLeave: (_) {
        if (_dropAt.value?.line == line) _dropAt.value = null;
      },
      onAcceptWithDetails: (d) {
        final at = _dropAt.value;
        _dropAt.value = null;
        onDrop(d.data.ids, at?.line == line ? at!.below : true);
      },
      builder: (context, _, __) => ValueListenableBuilder(
        valueListenable: _dropAt,
        child: child,
        builder: (context, at, child) => at == null || at.line != line
            ? child!
            : DecoratedBox(
                key: const ValueKey('template-drop-line'),
                position: DecorationPosition.foreground,
                decoration: BoxDecoration(
                  border:
                      at.below ? Border(bottom: edge) : Border(top: edge),
                ),
                child: child,
              ),
      ),
    );
  }

  // --- Picking ----------------------------------------------------------------

  /// A press on a control's row: pick it, or add it or run to it with Ctrl
  /// or Shift.
  void _press(String id) {
    final keys = HardwareKeyboard.instance;
    final order = [for (final c in _shown) c.id];
    setState(() {
      final from = _anchor == null ? -1 : order.indexOf(_anchor!);
      final to = order.indexOf(id);
      if (keys.isShiftPressed && from >= 0 && to >= 0) {
        final (a, b) = from <= to ? (from, to) : (to, from);
        _picked
          ..clear()
          ..addAll(order.sublist(a, b + 1));
      } else if (keys.isControlPressed || keys.isMetaPressed) {
        if (!_picked.remove(id)) _picked.add(id);
        _anchor = id;
      } else {
        _picked
          ..clear()
          ..add(id);
        _anchor = id;
      }
      _lead = id;
    });
  }

  /// The arrows: the pick moves a row, or grows with Shift.
  bool _step(int by) {
    final order = [for (final c in _shown) c.id];
    if (order.isEmpty) return false;
    // From the end the last press or step left, or the pick's own end.
    var at = _lead != null && _picked.contains(_lead)
        ? order.indexOf(_lead!)
        : -1;
    for (var i = 0; at < 0 && i < order.length; i++) {
      final from = by < 0 ? i : order.length - 1 - i;
      if (_picked.contains(order[from])) at = from;
    }
    final next = at < 0
        ? (by > 0 ? 0 : order.length - 1)
        : (at + by).clamp(0, order.length - 1);
    final from = _anchor == null ? -1 : order.indexOf(_anchor!);
    setState(() {
      if (HardwareKeyboard.instance.isShiftPressed && from >= 0) {
        final (a, b) = from <= next ? (from, next) : (next, from);
        _picked
          ..clear()
          ..addAll(order.sublist(a, b + 1));
      } else {
        _picked
          ..clear()
          ..add(order[next]);
        _anchor = order[next];
      }
      _lead = order[next];
    });
    _bringIntoView(order[next]);
    return true;
  }

  /// Scroll a control's row into view, as little as it takes.
  void _bringIntoView(String id) {
    final row = _rowKeys[id]?.currentContext;
    if (row == null) return;
    final box = row.findRenderObject();
    if (box is! RenderBox || !_scroll.hasClients) return;
    final view = _scroll.position;
    final viewport = view.context.storageContext.findRenderObject();
    if (viewport is! RenderBox) return;
    final top = box.localToGlobal(Offset.zero, ancestor: viewport).dy;
    final bottom = top + box.size.height;
    final policy = top < 0
        ? ScrollPositionAlignmentPolicy.keepVisibleAtStart
        : bottom > view.viewportDimension
            ? ScrollPositionAlignmentPolicy.keepVisibleAtEnd
            : null;
    if (policy == null) return;
    final reveal = ThemeScope.of(context).motion.reveal;
    Scrollable.ensureVisible(row,
        alignmentPolicy: policy,
        duration: reveal.isStill ? Duration.zero : reveal.duration,
        curve: reveal.curve);
  }

  void _startRename(BridgeControl control) {
    if (control.gone) return;
    setState(() {
      _renamingGroup = null;
      _renamingOption = null;
      _renaming = control.id;
    });
  }

  // --- Keys -------------------------------------------------------------------

  /// Whether the keyboard is in a text box, which keeps its own keys.
  bool get _typing {
    final focused = FocusManager.instance.primaryFocus?.context;
    return focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null);
  }

  bool _onKey(KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) return false;
    if (!mounted ||
        lumitModalOpen ||
        _ui.activePanel != Panel.graphicsTemplate ||
        _typing) {
      return false;
    }
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed || keys.isMetaPressed || keys.isAltPressed) {
      return false;
    }
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowUp:
        return _step(-1);
      case LogicalKeyboardKey.arrowDown:
        return _step(1);
      case LogicalKeyboardKey.enter ||
            LogicalKeyboardKey.numpadEnter ||
            LogicalKeyboardKey.f2:
        final first =
            _shown.where((c) => _picked.contains(c.id)).firstOrNull;
        if (first == null || first.gone) return false;
        if (event is KeyDownEvent) _startRename(first);
        return true;
    }
    return false;
  }

  /// Escape while a menu's option is being recorded: stop, and keep nothing.
  bool _escape() {
    if (!mounted ||
        lumitModalOpen ||
        _ui.activePanel != Panel.graphicsTemplate ||
        _template?.recording == null ||
        _typing) {
      return false;
    }
    _stopRecording(keep: false);
    return true;
  }

  // --- Menus that set several rows --------------------------------------------

  void _addMenu() {
    final comp = _comp;
    if (comp == null) return;
    String? made;
    final added = _try(() => made = comp.addMenuControl(
          label: l10n.templateMenu,
          options: [l10n.templateMenuOption(1), l10n.templateMenuOption(2)],
          layerName: l10n.templateControlsLayer,
        ));
    final id = made;
    if (!added || id == null) return;
    setState(() {
      _openMenus.add(id);
      _picked
        ..clear()
        ..add(id);
      _anchor = id;
      _relist();
    });
  }

  void _toggleMenu(String id) => setState(() {
        if (!_openMenus.remove(id)) _openMenus.add(id);
        _relist();
      });

  void _setOptions(BridgeControl control, List<String> options) {
    final comp = _comp;
    if (comp == null || options.isEmpty) return;
    _try(() => comp.setMenuOptions(id: control.id, options: options));
  }

  void _renameOption(BridgeControl control, int option, String text) {
    final name = text.trim();
    setState(() => _renamingOption = null);
    if (name.isEmpty ||
        option >= control.options.length ||
        name == control.options[option]) {
      return;
    }
    _setOptions(control, [
      for (var i = 0; i < control.options.length; i++)
        i == option ? name : control.options[i],
    ]);
  }

  /// Add an option, named by its place, and open its name for typing.
  void _addOption(BridgeControl control) {
    var n = control.options.length + 1;
    while (control.options.contains(l10n.templateMenuOption(n))) {
      n++;
    }
    final at = control.options.length;
    _setOptions(control, [...control.options, l10n.templateMenuOption(n)]);
    final now = _controls.where((c) => c.id == control.id).firstOrNull;
    if (now != null && now.options.length > at) {
      setState(() => _renamingOption = (control.id, at));
    }
  }

  void _removeOption(BridgeControl control, int option) {
    if (control.options.length < 2) return;
    _setOptions(control, [
      for (var i = 0; i < control.options.length; i++)
        if (i != option) control.options[i],
    ]);
  }

  void _record(BridgeControl control, int option) {
    final comp = _comp;
    if (comp == null) return;
    _try(() => comp.startRecording(id: control.id, option: option));
  }

  void _stopRecording({required bool keep}) {
    final comp = _comp;
    if (comp == null) return;
    _try(() => comp.stopRecording(keep: keep));
  }

  // --- The foot ---------------------------------------------------------------

  void _setMark(TemplateMark mark, int? frame) {
    final comp = _comp;
    final template = _template;
    if (comp == null || template == null) return;
    _try(
      () => comp.setTemplateMarks(
        poster: mark == TemplateMark.poster ? frame : template.poster,
        intro: mark == TemplateMark.intro ? frame : template.intro,
        outro: mark == TemplateMark.outro ? frame : template.outro,
      ),
      // The one thing the engine turns down here.
      refused:
          mark == TemplateMark.poster ? null : l10n.templateOutroBeforeIntro,
    );
  }

  // --- Right-click ------------------------------------------------------------

  /// Select [control]'s layer and unfold the Timeline to the row it sets.
  void _reveal(BridgeControl control) {
    final entry = _ui.model.layers
        .where((e) => e.layer.internallayerId == control.row.layer)
        .firstOrNull;
    if (entry == null) return;
    final fold = templateRowFold(control.row, entry);
    _ui.setSelection([entry.layer]);
    setPanelVisible(_ui.split, Panel.timeline, true);
    activatePanelTab(_ui.split, Panel.timeline);
    _ui.activePane.value = Panel.timeline.pane();
    _ui.workspace.touch();
    // Once the Timeline is up, which it may not have been.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _ui.revealRowRequest.value =
          (layer: control.row.layer, open: fold.open, row: fold.row);
    });
    WidgetsBinding.instance.ensureVisualUpdate();
  }

  void _controlMenu(BridgeControl control, Offset at) {
    final comp = _comp;
    if (comp == null) return;
    if (!_picked.contains(control.id)) {
      setState(() {
        _picked
          ..clear()
          ..add(control.id);
        _anchor = control.id;
      });
    }
    final picked = _picked.toList();
    // A control whose row has gone can only be removed.
    if (control.gone) {
      unawaited(showMenuEntriesAt(context, at, [
        MenuEntry(l10n.templateRemove, () => _remove(picked)),
      ]));
      return;
    }
    void textRows({bool? font, bool? size, bool? colour}) =>
        _try(() => comp.setControlTextRows(
              id: control.id,
              font: font ?? control.font,
              size: size ?? control.size,
              colour: colour ?? control.colour,
            ));
    unawaited(showMenuEntriesAt(context, at, [
      MenuEntry(l10n.rename, () => _startRename(control)),
      if (control.kind == BridgeControlKind.number)
        MenuEntry(
            l10n.templateRange,
            () => unawaited(showTemplateRange(context, at, control,
                onSet: (ranged, min, max) => _try(() => comp.setControlRange(
                    id: control.id, ranged: ranged, min: min, max: max))))),
      if (control.kind == BridgeControlKind.text)
        MenuEntry.submenu(l10n.templateTextRows, [
          MenuEntry(engineLabel('Font'), () => textRows(font: !control.font),
              checked: control.font),
          MenuEntry(engineLabel('Size'), () => textRows(size: !control.size),
              checked: control.size),
          MenuEntry(
              engineLabel('Colour'), () => textRows(colour: !control.colour),
              checked: control.colour),
        ]),
      MenuEntry.submenu(l10n.templateMoveToGroup, [
        for (final group in templateGroups(_controls))
          MenuEntry(group, () => _moveTo(picked, group: group),
              checked: picked.length == 1 && control.group == group),
        MenuEntry(l10n.templateNewGroup, () => _newGroup(picked)),
        MenuEntry(l10n.none, () => _moveTo(picked),
            checked: picked.length == 1 && control.group == null),
      ]),
      MenuEntry(l10n.templateRevealRow, () => _reveal(control)),
      MenuEntry.divider(),
      MenuEntry(l10n.templateRemove, () => _remove(picked)),
    ]));
  }

  void _groupMenu(String group, Offset at) =>
      unawaited(showMenuEntriesAt(context, at, [
        MenuEntry(l10n.rename, () => setState(() {
              _renaming = null;
              _renamingOption = null;
              _renamingGroup = group;
            })),
      ]));

  void _optionMenu(BridgeControl control, int option, Offset at) {
    final comp = _comp;
    if (comp == null) return;
    final sets = option < control.looks.length && control.looks[option] > 0;
    unawaited(showMenuEntriesAt(context, at, [
      MenuEntry(
          l10n.templateClearLook,
          sets
              ? () => _try(() => comp.clearLook(id: control.id, option: option))
              : null),
    ]));
  }

  // --- Drawing ----------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final comp = _comp;
    if (comp == null) {
      return PlaceholderPanel(
        icon: LumitIcon.comp,
        title: l10n.panelGraphicsTemplate,
        hint: l10n.selectACompositionFirst,
      );
    }
    // A node graph's Inputs are its controls, made in the graph.
    if (_graph) {
      return PlaceholderPanel(
        icon: LumitIcon.comp,
        title: l10n.panelGraphicsTemplate,
        hint: '',
      );
    }
    final template = _template;
    final body = template == null || template.controls.isEmpty
        ? PlaceholderPanel(
            key: const ValueKey('template-empty'),
            icon: LumitIcon.comp,
            title: l10n.templateNoControls,
            hint: '',
          )
        : _list(t);
    final foot = template == null
        ? null
        : TemplateFoot(
            template: template,
            fps: _ui.model.fpsExact,
            startFrame: _ui.model.startFrame,
            lastFrame: (_ui.model.durationFrames - 1).clamp(0, 1 << 31),
            onMark: _setMark,
            onMarkAtPlayhead: (mark) =>
                _setMark(mark, _ui.playheadFrame.value),
            onData: () => showTemplateDataWindow(context, comp),
            onSave: () => showSaveTemplateDialog(context, comp),
          );
    return LayoutBuilder(builder: (context, box) {
      // Too short for the list to keep a few rows beside the foot: the
      // whole panel scrolls instead, so nothing is cut off.
      if (box.maxHeight < _shortPanel) {
        return SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _bar(t, template),
              SizedBox(height: _listFloor, child: body),
              if (foot != null) foot,
            ],
          ),
        );
      }
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _bar(t, template),
          Expanded(child: body),
          if (foot != null) foot,
        ],
      );
    });
  }

  /// The least room the list keeps when the panel is short, and the height
  /// under which the panel scrolls whole.
  static const double _listFloor = 120;
  static const double _shortPanel = 300;

  /// The mark that says a menu's option is being recorded.
  Widget _recordMark(LumitTheme t) => Container(
        width: 7,
        height: 7,
        decoration: BoxDecoration(color: t.error, shape: BoxShape.circle),
      );

  /// A small word that is a button: Record, Stop.
  Widget _wordButton(LumitTheme t, String key, String word,
          VoidCallback? onPressed, {Color? ink}) =>
      HouseButton(
        key: ValueKey<String>(key),
        frameless: true,
        small: true,
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
        onPressed: onPressed,
        child: Text(word,
            style: t.small.copyWith(
                color: onPressed == null
                    ? t.textDisabled
                    : (ink ?? t.textMuted))),
      );

  Widget _bar(LumitTheme t, BridgeTemplate? template) {
    final recording = template?.recording != null;
    return Container(
      height: 26,
      color: t.surface1,
      padding: const EdgeInsetsDirectional.only(start: 10, end: 6),
      child: Row(
        children: [
          Expanded(
            child: Text(_compName,
                maxLines: 1, overflow: TextOverflow.ellipsis, style: t.small),
          ),
          // Saved to the library once, and changed here since.
          if (template != null && template.id != null && template.changed) ...[
            const SizedBox(width: 6),
            Container(
              key: const ValueKey('template-changed'),
              padding: const EdgeInsets.symmetric(horizontal: 4),
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(t.tokens.wellRadius),
                border: Border.all(color: t.hairlineStrong),
              ),
              child: Text(l10n.templateChanged,
                  style: t.small.copyWith(color: t.textMuted)),
            ),
          ],
          if (recording) ...[
            const SizedBox(width: 8),
            _recordMark(t),
            const SizedBox(width: 2),
            _wordButton(t, 'template-stop', l10n.templateStop,
                () => _stopRecording(keep: true),
                ink: t.textPrimary),
          ],
          const SizedBox(width: 4),
          captionsMark(
              t,
              'template-add',
              l10n.templateAdd,
              LumitIcons.addEffect,
              (context) => unawaited(showMenuEntriesAt(
                  context,
                  popupBelow(context),
                  [MenuEntry(l10n.templateMenu, _addMenu)],
                  width: 160))),
        ],
      ),
    );
  }

  Widget _list(LumitTheme t) => HouseScrollbar(
        controller: _scroll,
        child: SingleChildScrollView(
          key: const ValueKey('template-list'),
          controller: _scroll,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              // Above everything: first, and out of every group.
              _target(
                t,
                line: 'top',
                onDrop: (ids, _) => _dropOnTop(ids),
                child: const SizedBox(height: 4),
              ),
              // Keyed by what it is, so a row keeps its wells and its text
              // box when the rows round it move.
              for (final line in _lines)
                KeyedSubtree(
                  key: ValueKey<String>(_keyOf(line)),
                  child: _line(t, line),
                ),
              _target(
                t,
                line: 'end',
                below: (_) => false,
                onDrop: (ids, _) => _dropOnEnd(ids),
                child: const SizedBox(height: 28),
              ),
            ],
          ),
        ),
      );

  String _keyOf(TemplateLine line) => switch (line) {
        TemplateGroupLine(:final name) => 'group:$name',
        TemplateControlLine(:final control) => 'control:${control.id}',
        TemplateTextRowLine(:final control, :final which) =>
          '${which.name}:${control.id}',
        TemplateOptionLine(:final control, :final option) =>
          'option:${control.id}:$option',
        TemplateAddOptionLine(:final control) => 'add:${control.id}',
      };

  Widget _line(LumitTheme t, TemplateLine line) => switch (line) {
        TemplateGroupLine() => _groupLine(t, line),
        TemplateControlLine(:final control) => _controlLine(t, control),
        TemplateTextRowLine() => _textRowLine(t, line),
        TemplateOptionLine() => _optionLine(t, line),
        TemplateAddOptionLine(:final control) => _addOptionLine(t, control),
      };

  /// A line's box, as an Effect controls row has it: the hairline under it
  /// and the same room either side.
  Widget _box(LumitTheme t, Widget child, {Key? key, bool picked = false}) =>
      Container(
        key: key,
        decoration: BoxDecoration(
          color: picked ? t.selectionFill : null,
          border: Border(bottom: BorderSide(color: t.hairline)),
        ),
        padding: const EdgeInsets.fromLTRB(10, 2, 10, 2),
        child: child,
      );

  Widget _groupLine(LumitTheme t, TemplateGroupLine line) {
    final name = line.name;
    if (_renamingGroup == name) {
      return _box(
        t,
        key: ValueKey<String>('template-group-$name'),
        SizedBox(
          height: fxRowHeight(t),
          child: Row(
            children: [
              const SizedBox(width: fxKeyColumnWidth + 4),
              Expanded(
                child: TemplateRenameField(
                  initial: name,
                  onDone: (text) => _renameGroup(name, text),
                  onCancel: () => setState(() => _renamingGroup = null),
                ),
              ),
            ],
          ),
        ),
      );
    }
    return _target(
      t,
      line: 'group:$name',
      onDrop: (ids, _) => _dropOnGroup(ids, name),
      child: _box(
        t,
        key: ValueKey<String>('template-group-$name'),
        _DoubleClick(
          onDoubleClick: () => setState(() {
            _renaming = null;
            _renamingOption = null;
            _renamingGroup = name;
          }),
          child: fxGroupHeaderRow(
            context,
            label: name,
            open: line.open,
            onToggle: () => setState(() {
              if (!_shutGroups.remove(name)) _shutGroups.add(name);
              _relist();
            }),
            onContextMenu: (at) => _groupMenu(name, at),
          ),
        ),
      ),
    );
  }

  /// What a control's row carries before its name: a menu's twirl, or the
  /// mark of a row that is keyed.
  Widget? _mark(LumitTheme t, BridgeControl control) {
    if (control.gone) return null;
    if (control.kind == BridgeControlKind.menu) {
      final open = _openMenus.contains(control.id) ||
          _template?.recording?.menu == control.id;
      return GestureDetector(
        key: ValueKey<String>('template-twirl-${control.id}'),
        behavior: HitTestBehavior.opaque,
        onTap: () => _toggleMenu(control.id),
        child: HoverMark(
          builder: (context, lit) => TwirlTurn(
            open: open,
            child: lumitIcon(
              open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
              size: iconSize,
              color: open
                  ? t.textPrimary
                  : Color.lerp(t.textMuted, t.textPrimary, lit)!,
            ),
          ),
        ),
      );
    }
    if (control.animated) {
      return glyph.LumitIcon(stopwatchGlyph(t),
          key: ValueKey<String>('template-keyed-${control.id}'),
          size: iconSize,
          colour: t.animated);
    }
    return null;
  }

  Widget _controlLine(LumitTheme t, BridgeControl control) {
    final id = control.id;
    final picked = _picked.contains(id);
    final rowKey = _rowKeys.putIfAbsent(id, GlobalKey.new);
    final label = Text(
      // In the case a property's name is drawn in, as a copy's row draws it.
      t.propertyCase(control.label),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: control.gone
          ? t.body.copyWith(color: t.textDisabled)
          : (picked ? t.bodyPrimary : t.body),
    );
    final Widget name;
    if (_renaming == id) {
      name = TemplateRenameField(
        initial: control.label,
        onDone: (text) => _renameControl(control, text),
        onCancel: () => setState(() => _renaming = null),
      );
    } else if (control.gone) {
      name = label;
    } else {
      // The name is the row's handle: it drags the row, and every picked row
      // with it.
      final ids = picked
          ? [
              for (final c in _controls)
                if (_picked.contains(c.id)) c.id,
            ]
          : [id];
      name = Draggable<TemplateControlsDrag>(
        data: TemplateControlsDrag(ids),
        dragAnchorStrategy: pointerDragAnchorStrategy,
        onDragStarted: () => _narrowTo = null,
        onDragEnd: (_) => _dropAt.value = null,
        feedback: Container(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
          decoration: BoxDecoration(
            color: t.surface2,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(color: t.accent),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(control.label, style: t.small),
              if (ids.length > 1) ...[
                const SizedBox(width: 6),
                Text('${ids.length}',
                    style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
              ],
            ],
          ),
        ),
        child: _DoubleClick(
          onDoubleClick: () => _startRename(control),
          child: SizedBox(
            height: fxRowHeight(t),
            child: Align(
                alignment: AlignmentDirectional.centerStart, child: label),
          ),
        ),
      );
    }
    final row = fxTwoColumnRow(
      context: context,
      name: name,
      keyframeControls: _mark(t, control),
      // The words and the two pickers of a font may take a second line.
      grows: control.kind == BridgeControlKind.text ||
          control.kind == BridgeControlKind.font,
      control: TemplateValue(
        control: control,
        ownMenu: _ownMenus.contains(id),
        onSet: (value) => _setValue(control, value),
      ),
    );
    return _target(
      t,
      line: 'control:$id',
      accepts: (ids) => !ids.contains(id),
      below: (global) => _below(id, global),
      onDrop: (ids, below) => _dropOnControl(ids, control, below),
      child: Listener(
        onPointerDown: (event) {
          if (event.buttons != kPrimaryButton) return;
          final keys = HardwareKeyboard.instance;
          final plain = !keys.isShiftPressed &&
              !keys.isControlPressed &&
              !keys.isMetaPressed;
          if (picked && plain && _picked.length > 1) {
            _narrowTo = id;
          } else if (!picked || !plain) {
            _press(id);
          }
        },
        onPointerUp: (_) {
          if (_narrowTo == id) _press(id);
          _narrowTo = null;
        },
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onSecondaryTapUp: (d) => _controlMenu(control, d.globalPosition),
          child: _box(
            t,
            key: rowKey,
            picked: picked,
            KeyedSubtree(
                key: ValueKey<String>('template-control-$id'), child: row),
          ),
        ),
      ),
    );
  }

  /// One of the rows a copy shows under a text control, read and not set.
  Widget _textRowLine(LumitTheme t, TemplateTextRowLine line) {
    final text = _texts[line.control.id];
    final (label, value) = switch (line.which) {
      TemplateTextRow.font => (
          engineLabel('Font'),
          text == null
              ? null
              : Text(
                  [
                    if (text.style.family.isEmpty)
                      l10n.textFontBuiltIn
                    else
                      text.style.family,
                    text.style.face,
                  ].where((s) => s.isNotEmpty).join(' '),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: t.small.copyWith(color: t.textMuted)),
        ),
      TemplateTextRow.size => (
          engineLabel('Size'),
          text == null
              ? null
              : templateReadout(t, text.size.toStringAsFixed(1)),
        ),
      TemplateTextRow.colour => (
          engineLabel('Colour'),
          text == null
              ? null
              : templateSwatch(
                  t, [text.fill.r, text.fill.g, text.fill.b, text.fill.a]),
        ),
    };
    return _box(
      t,
      key: ValueKey<String>(
          'template-${line.which.name}-${line.control.id}'),
      fxTwoColumnRow(
        context: context,
        name: Padding(
          padding: const EdgeInsetsDirectional.only(start: _indent),
          child: Text(t.propertyCase(label),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: t.body.copyWith(color: t.textMuted)),
        ),
        control: value ?? Text('—', style: t.small),
      ),
    );
  }

  Widget _optionLine(LumitTheme t, TemplateOptionLine line) {
    final control = line.control;
    final option = line.option;
    final own = _ownMenus.contains(control.id);
    final recording = _template?.recording;
    final here = recording != null &&
        recording.menu == control.id &&
        recording.option == option;
    final sets = option < control.looks.length ? control.looks[option] : 0;
    final said =
        own ? control.options[option] : engineLabel(control.options[option]);
    final Widget name = _renamingOption == (control.id, option)
        ? TemplateRenameField(
            initial: control.options[option],
            onDone: (text) => _renameOption(control, option, text),
            onCancel: () => setState(() => _renamingOption = null),
          )
        : _DoubleClick(
            // Only a menu made here names its own options.
            onDoubleClick: own
                ? () => setState(() {
                      _renaming = null;
                      _renamingGroup = null;
                      _renamingOption = (control.id, option);
                    })
                : null,
            child: SizedBox(
              height: fxRowHeight(t),
              child: Align(
                alignment: AlignmentDirectional.centerStart,
                child: Text(said,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: t.small),
              ),
            ),
          );
    return _box(
      t,
      key: ValueKey<String>('template-option-${control.id}-$option'),
      FxRowHoverRegion(
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onSecondaryTapUp: (d) =>
              _optionMenu(control, option, d.globalPosition),
          child: SizedBox(
            height: fxRowHeight(t),
            child: Row(
              children: [
                const SizedBox(width: fxKeyColumnWidth + 4 + _indent),
                Expanded(child: name),
                if (sets > 0) ...[
                  const SizedBox(width: 6),
                  LumitTooltip(
                    message: l10n.templateSetsRows(sets),
                    child: templateReadout(t, '$sets'),
                  ),
                ],
                const SizedBox(width: 6),
                if (here) ...[
                  _recordMark(t),
                  const SizedBox(width: 2),
                  _wordButton(
                      t,
                      'template-stop-${control.id}-$option',
                      l10n.templateStop,
                      () => _stopRecording(keep: true),
                      ink: t.textPrimary),
                ] else
                  _wordButton(
                      t,
                      'template-record-${control.id}-$option',
                      l10n.templateRecord,
                      // One option is recorded at a time.
                      recording == null
                          ? () => _record(control, option)
                          : null),
                // Kept in the row where there is nothing to remove, so the
                // words before it line up down the menu.
                SizedBox(
                  width: 18,
                  child: own && control.options.length > 1
                      ? FxQuiet(
                          child: LumitTooltip(
                            message: l10n.templateRemoveOption,
                            child: HouseButton(
                              key: ValueKey<String>('template-option-remove-'
                                  '${control.id}-$option'),
                              frameless: true,
                              small: true,
                              padding: const EdgeInsets.all(1),
                              onPressed: () => _removeOption(control, option),
                              child: glyph.LumitIcon(LumitIcons.close,
                                  size: 12, colour: t.textMuted),
                            ),
                          ),
                        )
                      : null,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _addOptionLine(LumitTheme t, BridgeControl control) => _box(
        t,
        key: ValueKey<String>('template-option-add-${control.id}'),
        SizedBox(
          height: fxRowHeight(t),
          child: Row(
            children: [
              const SizedBox(width: fxKeyColumnWidth + _indent),
              HouseButton(
                key: ValueKey<String>('template-add-option-${control.id}'),
                frameless: true,
                small: true,
                padding:
                    const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
                onPressed: () => _addOption(control),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    glyph.LumitIcon(LumitIcons.addEffect,
                        size: 12, colour: t.textMuted),
                    const SizedBox(width: 4),
                    Text(l10n.templateAddOption,
                        style: t.small.copyWith(color: t.textMuted)),
                  ],
                ),
              ),
            ],
          ),
        ),
      );
}

/// Hears a double click on [child] without holding up a single one: the
/// first press goes where it was going, and the second calls
/// [onDoubleClick].
class _DoubleClick extends StatefulWidget {
  final VoidCallback? onDoubleClick;
  final Widget child;
  const _DoubleClick({required this.onDoubleClick, required this.child});

  @override
  State<_DoubleClick> createState() => _DoubleClickState();
}

class _DoubleClickState extends State<_DoubleClick> {
  Duration? _last;
  Offset _at = Offset.zero;

  @override
  Widget build(BuildContext context) => Listener(
        behavior: HitTestBehavior.opaque,
        onPointerDown: (event) {
          final heard = widget.onDoubleClick;
          if (heard == null || event.buttons != kPrimaryButton) return;
          final last = _last;
          final near = (event.position - _at).distance <= kDoubleTapSlop;
          _last = event.timeStamp;
          _at = event.position;
          if (last != null &&
              near &&
              event.timeStamp - last <= kDoubleTapTimeout) {
            _last = null;
            heard();
          }
        },
        child: widget.child,
      );
}
