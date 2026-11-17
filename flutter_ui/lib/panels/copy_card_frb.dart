// The card a copy of a graphics template shows in Effect controls.
//
// A copy is a Precomp layer that places a composition with controls, or a
// clip of one in the Cut timeline. What it has set is one staged effect
// instance, so the rows that key are the ordinary effect rows: stopwatch,
// keyframes, expressions, the row menu and the reset arrow. The words, the
// fonts and the footage do not key, and are set through the copy's own calls.
//
// With several copies of one template selected the card shows the first and
// writes every edit to all of them as one undo step.

import 'dart:math' as math;
import 'dart:ui' as ui show Image, PixelFormat, decodeImageFromPixels;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart'
    show textFontFaces, textFontFamilies;
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/template.dart';
import 'package:lumit_flutter/src/rust/api/template/copy.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/lumit_icon.dart';
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/copies.dart';
import '../state/drag_payloads.dart';
import '../state/template_library.dart';
import '../state/workspace.dart' show Workspace;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/share_marks.dart';
import 'control_rows.dart' show copyControlRow;
import 'effect_param_row_frb.dart';
import 'fx_key_strip.dart';
import 'fx_section.dart';
import 'graph_panel.dart' show graphCompById, graphProjectItems;
import 'layer_fold_frb.dart'
    show asOneUndoStep, clipFoldPrefix, effectPath, inputsPath;
import 'param_row_menu.dart' show ParamRowExtras, axisName;
import 'timeline_extras_frb.dart' show showMenuAt;

/// The rows that say whether a copy plays the template's intro and outro.
const String copyPlayIntro = 'play_intro';
const String copyPlayOutro = 'play_outro';

/// What stands in for a value that differs between the selected copies.
const String _dash = '—';

/// The longer edge a footage well's picture is read at.
const int _wellPictureEdge = 96;

/// One copy the card edits.
class CopyTarget {
  const CopyTarget({
    required this.layer,
    this.clip,
    required this.copy,
    required this.rows,
  });

  /// The Precomp layer, or the Sequence layer the clip is on.
  final LayerReference layer;

  /// The clip, for a copy in the Cut timeline. Null for a Precomp layer.
  final UuidValue? clip;
  final BridgeCopy copy;

  /// The copy's staged rows as they were last read.
  final BridgeEffectInstanceInfo rows;
}

/// A copy's card: the template's name, Open and Reset on its heading, and
/// its controls under their groups.
class CopyCardFrb extends StatefulWidget {
  const CopyCardFrb({
    super.key,
    required this.targets,
    required this.comp,
    required this.editor,
    required this.open,
    required this.onToggle,
    required this.isGroupOpen,
    required this.onToggleGroup,
    required this.allLayers,
    required this.playheadFrame,
    required this.onSeek,
    required this.onLive,
    required this.onChanged,
    this.filter = '',
  });

  /// The copies the card edits, the one it shows first. Never empty.
  final List<CopyTarget> targets;

  /// The composition the copies are in.
  final CompositionReference comp;

  /// The panel's staging, which a row's drag and its commit go through.
  final EffectStackEditor editor;
  final bool open;
  final VoidCallback onToggle;
  final bool Function(String path, bool collapsedByDefault) isGroupOpen;
  final void Function(String path, bool collapsedByDefault) onToggleGroup;
  final List<BridgeLayerEntry> allLayers;
  final int playheadFrame;
  final ValueChanged<int> onSeek;

  /// A drag tick on a row: preview it, do not commit it.
  final void Function(UuidValue effect, String param, BridgeEffectValue value)
      onLive;

  /// The document changed, so the panel reads it again.
  final VoidCallback onChanged;

  /// The panel's row filter, lowercase. Empty draws every control.
  final String filter;

  @override
  State<CopyCardFrb> createState() => _CopyCardFrbState();
}

class _CopyCardFrbState extends State<CopyCardFrb> {
  /// Each text control's box and its focus, by the control's id.
  final Map<String, TextEditingController> _words = {};
  final Map<String, FocusNode> _wordNodes = {};

  /// The words last sent to the engine for each control, so only a change
  /// is written.
  final Map<String, String> _sent = {};

  /// The text boxes drawn this build, in the card's order: what Tab walks.
  final List<String> _wordOrder = [];

  /// The box a press has just put the cursor in, whose words that press
  /// selects.
  String? _clickedInto;

  /// The project a stretch of typing holds an undo group open on, closed
  /// when the box loses the cursor.
  ProjectReference? _typing;

  /// The boxes to bring up to the document even while the cursor is in
  /// them: a Reset or a Set as default changed their words under it.
  final Set<String> _resync = {};

  /// The controls whose differing values have been opened to set.
  final Set<String> _revealed = {};

  /// The installed font families, and the faces of each asked about.
  List<String>? _families;
  bool _familiesAsked = false;
  final Map<String, List<String>> _faces = {};

  /// What each footage well shows, by the item's id: its name, and its
  /// picture once that is read.
  final Map<String, ({String name, ui.Image? picture})> _pictures = {};

  CopyTarget get _first => widget.targets.first;

  /// The templates the library has a newer version of: the heading offers
  /// Update while this copy's is one.
  late final TemplateUpdates _updates;

  void _updatesChanged() {
    if (mounted) setState(() {});
  }

  Future<void> _updateTemplate() async {
    await _updates.update([_first.copy.comp]);
    if (mounted) widget.onChanged();
  }

  LumitState get _app => Provider.of<LumitState>(context, listen: false);

  @override
  void initState() {
    super.initState();
    copyWordsFocus.addListener(_onFocusAsked);
    _updates = TemplateUpdates.of(
        _app, Provider.of<LumitUiState>(context, listen: false))
      ..addListener(_updatesChanged);
    _readOutside();
    _syncWords();
    WidgetsBinding.instance.addPostFrameCallback((_) => _onFocusAsked());
  }

  @override
  void didUpdateWidget(covariant CopyCardFrb old) {
    super.didUpdateWidget(old);
    final was = old.targets.first;
    if (old.targets.length != widget.targets.length ||
        was.layer.internallayerId != _first.layer.internallayerId ||
        was.clip != _first.clip) {
      _revealed.clear();
    }
    _readOutside();
    // The document has been read again since any edit made from here.
    _syncWords(fresh: true);
    WidgetsBinding.instance.addPostFrameCallback((_) => _onFocusAsked());
  }

  @override
  void dispose() {
    copyWordsFocus.removeListener(_onFocusAsked);
    _updates.removeListener(_updatesChanged);
    _endTyping();
    for (final box in _words.values) {
      box.dispose();
    }
    for (final node in _wordNodes.values) {
      node.dispose();
    }
    for (final held in _pictures.values) {
      held.picture?.dispose();
    }
    super.dispose();
  }

  // --- Reading --------------------------------------------------------------

  Map<String, BridgeEffectValue> _valuesOf(CopyTarget target) =>
      {for (final v in target.rows.values) v.id: v.value};

  /// Whether [rows] hold different values on the selected copies.
  bool _mixed(Iterable<String> rows) {
    if (widget.targets.length < 2) return false;
    final first = _valuesOf(_first);
    for (final other in widget.targets.skip(1)) {
      final values = _valuesOf(other);
      for (final row in rows) {
        if (values[row] != first[row]) return true;
      }
    }
    return false;
  }

  /// Whether a row holds something other than the template's own value,
  /// which is the row's declared default: the copy set it, or its data did.
  bool _differs(BridgeParamInfo? param, BridgeEffectValue? value) {
    if (param == null) return false;
    final fallback = defaultEffectValue(param.kind);
    return fallback != null &&
        !isDefaultEffectValue(value, fallback,
            owner: _first.layer.internallayerId);
  }

  static String _textOf(BridgeEffectValue? value) => switch (value) {
        BridgeEffectValue_Text(:final field0) => field0,
        _ => '',
      };

  /// A font row's family and face: the row holds the family, a line break,
  /// the face.
  static (String, String) _fontOf(BridgeEffectValue? value) {
    final text = _textOf(value);
    final cut = text.indexOf('\n');
    return cut < 0
        ? (text, '')
        : (text.substring(0, cut), text.substring(cut + 1));
  }

  /// What the card shows that the document does not hold: the installed
  /// fonts, and the names and pictures of the items in the footage wells.
  /// Asked for here, never from a build.
  void _readOutside() {
    final values = _valuesOf(_first);
    final fonts = <String>[];
    final items = <String>[];
    for (final c in _first.copy.controls) {
      for (final row in c.rows) {
        final isFont = c.kind == BridgeControlKind.font
            ? row == c.id
            : row == '${c.id}_font';
        if (isFont) fonts.add(_fontOf(values[row]).$1);
      }
      if (c.kind == BridgeControlKind.footage) {
        final item = _textOf(values[c.id]);
        if (item.isNotEmpty) items.add(item);
      }
    }
    if (fonts.isNotEmpty && !_familiesAsked) {
      _familiesAsked = true;
      textFontFamilies().then((families) {
        if (mounted) setState(() => _families = families);
      }).catchError((Object _) {
        // No list is a picker holding the built-in font alone.
        if (mounted) setState(() => _families = const []);
      });
    }
    for (final family in fonts) {
      if (family.isEmpty || _faces.containsKey(family)) continue;
      _faces[family] = const [];
      textFontFaces(family: family).then((faces) {
        if (mounted) setState(() => _faces[family] = faces);
      }).catchError((Object _) {});
    }
    final wanted = [
      for (final item in items)
        if (!_pictures.containsKey(item)) item,
    ];
    if (wanted.isEmpty) return;
    List<ItemReference> all;
    try {
      all = graphProjectItems(_app.project);
    } catch (_) {
      // The project went under the card, which then names no picture.
      return;
    }
    for (final id in wanted) {
      for (final item in all) {
        switch (item) {
          case ItemReference_Footage(:final field0)
              when '${field0.internalid}' == id:
            _pictures[id] = (name: _nameOf(item), picture: null);
            field0
                .thumbnail(maxEdge: _wellPictureEdge, frame: 0)
                .then((frame) {
              if (!mounted || frame == null) return;
              if (frame.width == 0 || frame.height == 0) return;
              ui.decodeImageFromPixels(frame.rgba, frame.width, frame.height,
                  ui.PixelFormat.rgba8888, (image) => _keepPicture(id, image));
            }).catchError((Object _) {});
          case ItemReference_Composition(:final field0)
              when '${field0.internalid}' == id:
            _pictures[id] = (name: _nameOf(item), picture: null);
            var frame = 0;
            try {
              frame = field0.durationFrames() ~/ 2;
            } catch (_) {
              // Deleted a moment ago: its first frame, if it has one.
            }
            Workspace.compStill(field0, frame, maxEdge: _wellPictureEdge)
                .then((image) {
              if (image != null) _keepPicture(id, image);
            }, onError: (_) {});
          default:
        }
      }
      // An item the project no longer has is asked about once, not again.
      _pictures.putIfAbsent(id, () => (name: '', picture: null));
    }
  }

  String _nameOf(ItemReference item) {
    try {
      return item.name();
    } catch (_) {
      return '';
    }
  }

  void _keepPicture(String id, ui.Image image) {
    final held = _pictures[id];
    if (!mounted || held == null) {
      image.dispose();
      return;
    }
    held.picture?.dispose();
    setState(() => _pictures[id] = (name: held.name, picture: image));
  }

  // --- Writing --------------------------------------------------------------

  /// Run [edit] on every selected copy as one undo step, then have the
  /// panel read the document again.
  void _toEach(void Function(CopyTarget target) edit) {
    void all() {
      for (final target in widget.targets) {
        try {
          edit(target);
        } catch (_) {
          // A copy that has gone, or a locked layer refusing: the others
          // still take the edit.
        }
      }
    }

    if (widget.targets.length > 1) {
      asOneUndoStep(_app.project, all);
    } else {
      all();
    }
    widget.onChanged();
  }

  /// A row's typed value or the release of its drag, on every selected copy.
  void _write(UuidValue effect, String param, BridgeEffectValue value) =>
      _writeAll(effect, {param: value});

  void _writeAll(UuidValue effect, Map<String, BridgeEffectValue> values) {
    void all() {
      widget.editor.writeAll(_first.layer, effect, values);
      for (final other in widget.targets.skip(1)) {
        try {
          final staged = other.clip == null
              ? other.layer.getGraphInputs()
              : other.layer.getClipInputs(clip: other.clip!, onCompClock: true);
          if (staged == null) continue;
          for (final entry in values.entries) {
            staged.setValue(id: entry.key, value: entry.value);
          }
          other.layer.setEffects(effects: [staged], clip: other.clip);
        } catch (_) {
          // That copy has gone or refused. The rest keep the value.
        }
      }
    }

    if (widget.targets.length > 1) {
      asOneUndoStep(_app.project, all);
    } else {
      all();
    }
    widget.onChanged();
  }

  /// Let [rows] follow the template again, on every selected copy.
  void _clear(List<String> rows) {
    // Every row, for the card's own Reset.
    _resync.addAll(rows.isEmpty ? _words.keys : rows);
    _toEach((t) => t.layer.clearCopyRows(rows: rows, clip: t.clip));
  }

  /// Write the shown copy's value for [control] into the template.
  void _setAsDefault(BridgeCopyControl control) {
    _resync.add(control.id);
    try {
      _first.layer.setCopyAsDefault(control: control.id, clip: _first.clip);
    } catch (_) {
      // The control went from the template under the menu.
    }
    widget.onChanged();
  }

  /// Front the template's composition, as a double-click on a Precomp
  /// layer's bar does.
  void _openTemplate() {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final CompositionReference? template;
    try {
      template = graphCompById(_app.project, _first.copy.comp);
    } catch (_) {
      return;
    }
    if (template == null) return;
    if (_first.clip == null) {
      ui.openNestedComp(_first.layer, template);
    } else {
      ui.setSelectedComp(template);
    }
  }

  // --- Words ----------------------------------------------------------------

  /// Make a box for each text control and bring each up to the document,
  /// leaving the one being typed in alone.
  void _syncWords({bool fresh = false}) {
    final values = _valuesOf(_first);
    final live = <String>{};
    for (final c in _first.copy.controls) {
      if (c.kind != BridgeControlKind.text) continue;
      live.add(c.id);
      final text = _mixed([c.id]) ? '' : _textOf(values[c.id]);
      final node = _wordNodes.putIfAbsent(c.id, () {
        final node = FocusNode(debugLabel: 'copy words');
        node.addListener(() => _onWordsFocus(c.id));
        return node;
      });
      final box = _words[c.id];
      if (box == null) {
        _sent[c.id] = text;
        _words[c.id] = TextEditingController(text: text)
          ..addListener(() => _onWords(c.id));
      } else if (((fresh && _resync.contains(c.id)) || !node.hasFocus) &&
          box.text != text) {
        _sent[c.id] = text;
        box.value = TextEditingValue(text: text);
      }
    }
    for (final gone in _words.keys.where((id) => !live.contains(id)).toList()) {
      _words.remove(gone)?.dispose();
      _wordNodes.remove(gone)?.dispose();
      _sent.remove(gone);
    }
    if (fresh) _resync.clear();
  }

  /// A box changed: its words go to every selected copy. One stretch of
  /// typing is one undo step.
  void _onWords(String id) {
    final box = _words[id];
    if (box == null) return;
    final same = box.text == _sent[id];
    // The press that put the cursor here selects the words: the caret that
    // press sets down is widened to all of them. A drag from the press is a
    // selection of its own and is left as it is.
    if (_clickedInto == id && same) {
      final picked = box.selection;
      if (picked.isValid && picked.isCollapsed && box.text.isNotEmpty) {
        box.selection =
            TextSelection(baseOffset: 0, extentOffset: box.text.length);
      }
      return;
    }
    if (_clickedInto == id) _clickedInto = null;
    if (same) return;
    final text = box.text;
    _sent[id] = text;
    _beginTyping();
    _toEach((t) => t.layer.setCopyValue(
        control: id, value: BridgeControlValue.text(text), clip: t.clip));
  }

  /// The press that put the cursor in a box has ended, so the next caret set
  /// down in it stays where it is put.
  void _pressOver() => Future.microtask(() => _clickedInto = null);

  void _onWordsFocus(String id) {
    final node = _wordNodes[id];
    final box = _words[id];
    if (node == null || box == null) return;
    if (node.hasFocus) {
      box.selection =
          TextSelection(baseOffset: 0, extentOffset: box.text.length);
      return;
    }
    _clickedInto = null;
    _endTyping();
    // The box shows what the document holds again.
    if (mounted) setState(_syncWords);
  }

  void _beginTyping() {
    if (_typing != null) return;
    final project = _app.project;
    if (project == null) return;
    project.beginUndoGroup();
    _typing = project;
  }

  void _endTyping() {
    _typing?.endUndoGroup();
    _typing = null;
  }

  /// Tab and Shift+Tab move between the card's text boxes in order. Past the
  /// first or the last the key is left to move on as it does anywhere.
  KeyEventResult _onWordsKey(String id, KeyEvent event) {
    if (event is KeyUpEvent || event.logicalKey != LogicalKeyboardKey.tab) {
      return KeyEventResult.ignored;
    }
    final at = _wordOrder.indexOf(id);
    final to = at + (HardwareKeyboard.instance.isShiftPressed ? -1 : 1);
    if (at < 0 || to < 0 || to >= _wordOrder.length) {
      return KeyEventResult.ignored;
    }
    _wordNodes[_wordOrder[to]]?.requestFocus();
    return KeyEventResult.handled;
  }

  String _groupPath(String group) => 'copy-group-${_first.copy.comp}-$group';

  /// A double-click on this copy asked for its first text box: the card and
  /// the group the box is in are opened, and the box takes the cursor.
  void _onFocusAsked() {
    final asked = copyWordsFocus.value;
    if (asked == null || !mounted) return;
    if (asked.layer != _first.layer.internallayerId ||
        asked.clip != _first.clip) {
      return;
    }
    copyWordsFocus.value = null;
    BridgeCopyControl? words;
    for (final c in _first.copy.controls) {
      if (c.kind == BridgeControlKind.text) {
        words = c;
        break;
      }
    }
    if (words == null) return;
    if (!widget.open) widget.onToggle();
    final group = words.group;
    if (group != null &&
        group.isNotEmpty &&
        !widget.isGroupOpen(_groupPath(group), false)) {
      widget.onToggleGroup(_groupPath(group), false);
    }
    _focusOnceBuilt(words.id, 3);
  }

  void _focusOnceBuilt(String id, int tries) {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      final node = _wordNodes[id];
      final at = node?.context;
      if (node == null || at == null) {
        if (tries > 0) _focusOnceBuilt(id, tries - 1);
        return;
      }
      node.requestFocus();
      Scrollable.ensureVisible(at, alignment: 0.1);
    });
    WidgetsBinding.instance.ensureVisualUpdate();
  }

  // --- Drawing --------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final first = _first;
    final copy = first.copy;
    final id = first.rows.id;
    final params = {
      for (final p
          in relistedRows(first.rows.derivedParams, first.rows.rowOptions))
        p.id: p,
    };
    final values = _valuesOf(first);
    BridgeEffectValue? shown(String row) =>
        widget.editor.stagedValue(id, row) ?? values[row];

    // The row filter: a heading that holds the text keeps every control, and
    // a card with no control that does is left out.
    final needle = widget.filter;
    final whole = needle.isEmpty || copy.name.toLowerCase().contains(needle);
    final controls = [
      for (final c in copy.controls)
        if (whole || c.label.toLowerCase().contains(needle)) c,
    ];
    if (!whole && controls.isEmpty) return const SizedBox.shrink();

    // What the rows are called in a shared project: the path the Timeline
    // gives them, so a row in hand in either panel is marked in both.
    final share = ShareRows.maybeOf(context);
    final named = share != null && share.active ? share : null;
    final owner = '${first.layer.internallayerId}';
    final base = first.clip == null
        ? inputsPath(owner)
        : effectPath(clipFoldPrefix(first.clip!), '$id');
    Widget marked(String row, Widget child) =>
        named == null ? child : named.row('$base/$row', child);

    final draw = _Draw(
      t: t,
      id: id,
      params: params,
      shown: shown,
      marked: marked,
      rowPath: first.clip == null ? base : null,
    );

    _wordOrder.clear();
    final rows = <Widget>[];
    final groups = <String, List<BridgeCopyControl>>{};
    for (final c in controls) {
      final group = c.group;
      if (group != null && group.isNotEmpty) {
        (groups[group] ??= []).add(c);
      } else {
        rows.addAll(_controlRows(context, draw, c));
      }
    }
    for (final entry in groups.entries) {
      final path = _groupPath(entry.key);
      final open = needle.isNotEmpty || widget.isGroupOpen(path, false);
      rows.add(Column(
        key: ValueKey<String>(path),
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          fxGroupHeaderRow(
            context,
            label: entry.key,
            open: open,
            onToggle: () => widget.onToggleGroup(path, false),
          ),
          if (open)
            for (final c in entry.value) ..._controlRows(context, draw, c),
        ],
      ));
    }
    final intro = params[copyPlayIntro];
    final outro = params[copyPlayOutro];
    if (needle.isEmpty && (intro != null || outro != null)) {
      rows.add(_playRow(context, draw, intro, outro));
    }

    // Reset is offered while the copy holds anything of its own.
    final own = widget.targets.any((t) => t.copy.controls.any((c) => c.own)) ||
        _differs(intro, shown(copyPlayIntro)) ||
        _differs(outro, shown(copyPlayOutro));

    return FxSection(
      title: copy.name,
      open: widget.open || needle.isNotEmpty,
      onToggle: widget.onToggle,
      twirlKey: ValueKey<String>('fx-twirl-$id'),
      theirs: named == null ? const [] : named.under(base),
      summaryKeys: [
        for (final v in first.rows.values)
          if (v.value case BridgeEffectValue_Float(
              field0: BridgeScalar_Keyframed(:final field0)))
            ...field0,
      ],
      actions: [
        if (copy.dataRows > 0)
          _RowStepper(
            key: ValueKey<String>('copy-row-$id'),
            row: copy.row,
            count: copy.dataRows,
            onFollow: (row) => _toEach(
                (t) => t.layer.followDataRow(row: row, clip: t.clip)),
          ),
        if (_updates.forComp(copy.comp) != null)
          _headingWord(t, 'copy-update', l10n.templateUpdate, _updateTemplate,
              colour: t.accent),
        _headingWord(t, 'copy-open', l10n.templateCopyOpen, _openTemplate),
        if (own)
          _headingWord(t, 'copy-reset', l10n.reset, () => _clear(const [])),
      ],
      onContextMenu: (at) => showMenuAt<void>(
        context: context,
        position: at,
        rows: (close) => [
          if (_updates.forComp(copy.comp) != null)
            MenuRow(
              key: const ValueKey<String>('copy-menu-update'),
              onPressed: () {
                close(null);
                _updateTemplate();
              },
              child: MenuLabel(l10n.templateUpdateMenu),
            ),
          MenuRow(
            key: const ValueKey<String>('copy-menu-open'),
            onPressed: () {
              close(null);
              _openTemplate();
            },
            child: MenuLabel(l10n.templateCopyOpen),
          ),
          MenuRow(
            key: const ValueKey<String>('copy-menu-reset'),
            enabled: own,
            onPressed: () {
              close(null);
              _clear(const []);
            },
            child: MenuLabel(l10n.reset, enabled: own),
          ),
        ],
      ),
      rows: rows,
    );
  }

  /// One of the heading's words: Update, Open, Reset.
  Widget _headingWord(
          LumitTheme t, String key, String label, VoidCallback onPressed,
          {Color? colour}) =>
      HouseButton(
        key: ValueKey<String>(key),
        frameless: true,
        small: true,
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
        onPressed: onPressed,
        child: Text(t.propertyCase(label),
            style: t.mono.copyWith(fontSize: 10, color: colour ?? t.textMuted)),
      );

  /// The rows one control draws, by its kind.
  List<Widget> _controlRows(
      BuildContext context, _Draw draw, BridgeCopyControl c) {
    final keyed = c.kind != BridgeControlKind.text &&
        c.kind != BridgeControlKind.font &&
        c.kind != BridgeControlKind.footage;
    // A value that differs between the selected copies is a dash until it
    // is opened to set.
    if (keyed && _mixed(c.rows) && !_revealed.contains(c.id)) {
      return [
        fxTwoColumnRow(
          context: context,
          name: _label(draw.t, c.label, false),
          control: _dashButton(draw.t, 'copy-mixed-${c.id}',
              () => setState(() => _revealed.add(c.id))),
        ),
      ];
    }
    switch (c.kind) {
      case BridgeControlKind.number ||
            BridgeControlKind.angle ||
            BridgeControlKind.colour ||
            BridgeControlKind.checkbox ||
            BridgeControlKind.menu:
        final param = draw.params[c.id];
        return [if (param != null) _row(draw, c, param)];
      case BridgeControlKind.point:
        return _pointRows(draw, c);
      case BridgeControlKind.text:
        final font = draw.params['${c.id}_font'];
        final size = draw.params['${c.id}_size'];
        final colour = draw.params['${c.id}_colour'];
        return [
          _wordsRow(context, draw, c),
          if (font != null) _fontRow(context, draw, c, font, under: true),
          if (size != null) _row(draw, c, _under(size)),
          if (colour != null) _row(draw, c, _under(colour)),
        ];
      case BridgeControlKind.font:
        final param = draw.params[c.id];
        return [
          if (param != null) _fontRow(context, draw, c, param, under: false),
        ];
      case BridgeControlKind.footage:
        return [_wellRow(context, draw, c)];
    }
  }

  /// [param] under a name set in from the labels above it: a text control's
  /// size and colour, under its words. The indent is in the name, so the
  /// value keeps the column every other row's value is in.
  BridgeParamInfo _under(BridgeParamInfo param) => BridgeParamInfo(
        id: param.id,
        label: ' ${engineLabel(param.label)}',
        kind: param.kind,
        unit: param.unit,
        derived: param.derived,
      );

  /// The control of the template that keeps [row], which a template placed
  /// inside another passes up as a control of its own. A clip's rows cannot
  /// be passed up.
  BridgeRowRef? _controlRow(String row) => _first.clip != null
      ? null
      : copyControlRow(_first.layer.internallayerId, _first.copy, row);

  /// One ordinary row of the staged instance. A row the copy has set draws
  /// its name in full ink, and one that follows the template in the quieter
  /// ink, as its value does at its default.
  Widget _row(_Draw draw, BridgeCopyControl c, BridgeParamInfo param) {
    final value = draw.shown(param.id);
    return _withDefault(
      c,
      draw.marked(
        param.id,
        EffectParamRowFrb(
          key: ValueKey<String>('copy-row-${draw.id}-${param.id}'),
          controlRow: _controlRow(param.id),
          effectId: draw.id,
          param: param,
          value: value,
          comp: widget.comp,
          ownerLayerId: _first.layer.internallayerId,
          ownerLayers: widget.allLayers,
          playheadFrame: widget.playheadFrame,
          onSeek: widget.onSeek,
          onWrite: _write,
          onLive: widget.onLive,
          onWritePair: _writeAll,
          twoColumn: true,
          graphColour: _differs(param, value) || c.fed
              ? draw.t.textPrimary
              : null,
          rowPath:
              draw.rowPath == null ? null : '${draw.rowPath}/${param.id}',
        ),
      ),
    );
  }

  /// A point: its rows across and down as one row, and its depth under them.
  List<Widget> _pointRows(_Draw draw, BridgeCopyControl c) {
    final x = draw.params['${c.id}_x'];
    final y = draw.params['${c.id}_y'];
    final z = draw.params['${c.id}_z'];
    BridgeParamInfo axis(BridgeParamInfo p, int i) => BridgeParamInfo(
          id: p.id,
          label: '${engineLabel(p.label)} ${axisName(i)}',
          kind: p.kind,
          unit: p.unit,
          derived: p.derived,
        );
    final pair = x != null &&
        y != null &&
        x.kind is BridgeParamKind_Float &&
        y.kind is BridgeParamKind_Float &&
        !drivenByExpression(draw.shown(x.id)) &&
        !drivenByExpression(draw.shown(y.id));
    return [
      if (pair)
        _withDefault(
          c,
          draw.marked(
            x.id,
            EffectPointRowFrb(
              key: ValueKey<String>('copy-row-${draw.id}-${x.id}-pair'),
              controlRow: _controlRow(x.id),
              ownerLayerId: _first.layer.internallayerId,
              effectId: draw.id,
              xParam: x,
              yParam: y,
              xValue: draw.shown(x.id),
              yValue: draw.shown(y.id),
              comp: widget.comp,
              playheadFrame: widget.playheadFrame,
              onSeek: widget.onSeek,
              onWrite: _write,
              onLive: widget.onLive,
              onWritePair: _writeAll,
              twoColumn: true,
              canSetExpression: true,
            ),
          ),
        )
      else ...[
        if (x != null) _row(draw, c, axis(x, 0)),
        if (y != null) _row(draw, c, axis(y, 1)),
      ],
      if (z != null) _row(draw, c, axis(z, 2)),
    ];
  }

  /// [row] with Set as default in its menu, on a control the copy has set.
  Widget _withDefault(BridgeCopyControl c, Widget row) => c.own
      ? ParamRowExtras(rows: (close) => [_defaultRow(c, close)], child: row)
      : row;

  MenuRow _defaultRow(BridgeCopyControl c, VoidCallback close) => MenuRow(
        key: ValueKey<String>('copy-menu-default-${c.id}'),
        onPressed: () {
          close();
          _setAsDefault(c);
        },
        child: MenuLabel(l10n.templateCopySetAsDefault),
      );

  /// What the menu of a row the card draws itself holds: Reset, and Set as
  /// default on a control the copy has set.
  List<Widget> _ownRows(BridgeCopyControl c, List<String> rows, bool own,
          VoidCallback close) =>
      [
        MenuRow(
          key: ValueKey<String>('copy-menu-reset-${rows.first}'),
          enabled: own,
          onPressed: () {
            close();
            _clear(rows);
          },
          child: MenuLabel(l10n.reset, enabled: own),
        ),
        if (c.own && c.kind != BridgeControlKind.footage) _defaultRow(c, close),
      ];

  /// The menu of a row the card draws itself: the words, a font, a well.
  Widget _menued(BuildContext context, BridgeCopyControl c,
          {required List<String> rows,
          required bool own,
          required Widget child}) =>
      GestureDetector(
        behavior: HitTestBehavior.opaque,
        onSecondaryTapUp: (d) => showMenuAt<void>(
          context: context,
          position: d.globalPosition,
          rows: (close) => _ownRows(c, rows, own, () => close(null)),
        ),
        child: child,
      );

  Widget _label(LumitTheme t, String label, bool own, {bool under = false}) =>
      Padding(
        padding: EdgeInsetsDirectional.only(start: under ? 11 : 0),
        child: Text(
          t.propertyCase(label),
          style: own ? t.bodyPrimary : t.body,
          overflow: TextOverflow.ellipsis,
        ),
      );

  /// The arrow that lets a row follow the template again, as the ordinary
  /// rows' own arrow does.
  Widget _resetArrow(LumitTheme t, String key, VoidCallback onPressed) =>
      LumitTooltip(
        message: l10n.tipResetParameter,
        child: HouseButton(
          key: ValueKey<String>(key),
          frameless: true,
          small: true,
          padding: const EdgeInsets.all(1),
          onPressed: onPressed,
          child: LumitIcon(LumitIcons.reset, size: 12, colour: t.textMuted),
        ),
      );

  Widget _dashButton(LumitTheme t, String key, VoidCallback onPressed) =>
      HouseButton(
        key: ValueKey<String>(key),
        frameless: true,
        small: true,
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
        onPressed: onPressed,
        child: Text(_dash, style: t.mono.copyWith(color: t.textMuted)),
      );

  /// A text control: its name, then a box as wide as the card that grows
  /// with its lines.
  Widget _wordsRow(BuildContext context, _Draw draw, BridgeCopyControl c) {
    final t = draw.t;
    final own = _differs(draw.params[c.id], draw.shown(c.id));
    final mixed = _mixed([c.id]);
    final box = _words[c.id];
    final node = _wordNodes[c.id];
    if (box == null || node == null) return const SizedBox.shrink();
    _wordOrder.add(c.id);
    final style = own || mixed || c.fed ? t.bodyPrimary : t.body;
    final strip = FxKeyStripScope.widthOf(context);
    return draw.marked(
      c.id,
      _menued(
        context,
        c,
        rows: [c.id],
        own: own,
        child: Container(
          key: ValueKey<String>('copy-words-row-${c.id}'),
          margin: EdgeInsetsDirectional.only(end: strip),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              SizedBox(
                height: fxRowHeight(t),
                child: Row(
                  children: [
                    const SizedBox(width: fxKeyColumnWidth + 4),
                    Expanded(child: _label(t, c.label, own || c.fed)),
                    if (own)
                      _resetArrow(
                          t, 'copy-reset-${c.id}', () => _clear([c.id])),
                  ],
                ),
              ),
              Padding(
                padding: const EdgeInsetsDirectional.only(
                    start: fxKeyColumnWidth + 4, bottom: 3),
                // Told of the press before the box takes the cursor, so
                // that press can select the words.
                child: Listener(
                  onPointerDown: (_) {
                    if (!node.hasFocus) _clickedInto = c.id;
                  },
                  // Once the press is over and the box has heard of it.
                  onPointerUp: (_) => _pressOver(),
                  onPointerCancel: (_) => _pressOver(),
                  child: Focus(
                    canRequestFocus: false,
                    skipTraversal: true,
                    onKeyEvent: (_, event) => _onWordsKey(c.id, event),
                    child: _WordsBox(
                      key: ValueKey<String>('copy-words-${c.id}'),
                      box: box,
                      node: node,
                      style: style,
                      hint: mixed ? _dash : null,
                      // A right-click in the box: the text's own menu, with
                      // the control's rows under it.
                      menuRows: (close) => _ownRows(c, [c.id], own, close),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  /// A font: the Text panel's family and face pickers. [under] sets it in
  /// under a text control's words.
  Widget _fontRow(BuildContext context, _Draw draw, BridgeCopyControl c,
      BridgeParamInfo param,
      {required bool under}) {
    final t = draw.t;
    final value = draw.shown(param.id);
    final (family, face) = _fontOf(value);
    final own = _differs(param, value);
    final mixed = _mixed([param.id]);

    void set(String family, String face) => _toEach((target) => under
        ? target.layer.setCopyFont(
            row: param.id, family: family, face: face, clip: target.clip)
        : target.layer.setCopyValue(
            control: c.id,
            value: BridgeControlValue.font(family: family, face: face),
            clip: target.clip));

    // The built-in font first, then every installed family. A family the
    // copy names and this machine lacks is listed too.
    final installed = _families ?? const <String>[];
    final missing = family.isNotEmpty && !installed.contains(family);
    final names = ['', ...installed, if (missing) family];
    final labels = [
      l10n.textFontBuiltIn,
      ...installed,
      if (missing) l10n.textFontMissing(family),
    ];
    final faces = _faces[family] ?? const <String>[];
    final regular = faces.firstWhere(
      (f) => f.toLowerCase() == 'regular',
      orElse: () => faces.isEmpty ? l10n.textFaceRegular : faces.first,
    );
    final shownFace = face.isEmpty ? regular : face;

    return draw.marked(
      param.id,
      _menued(
        context,
        c,
        rows: [param.id],
        own: own,
        child: fxTwoColumnRow(
          context: context,
          name: _label(
              t,
              under ? engineLabel(param.label) : c.label,
              own || mixed || c.fed,
              under: under),
          control: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Flexible(
                child: BareSearchDropdown(
                  key: ValueKey<String>('copy-family-${param.id}'),
                  value: mixed ? -1 : names.indexOf(family),
                  options: labels,
                  hint: l10n.textFontSearch,
                  familyOf: (i) => i == 0
                      ? 'Inter'
                      : (i <= installed.length ? names[i] : null),
                  // A new family starts on its regular face, since the old
                  // family's face may not be in it.
                  onChanged: (i) => set(names[i], ''),
                ),
              ),
              const SizedBox(width: 4),
              Flexible(
                child: BareDropdown<String>(
                  key: ValueKey<String>('copy-face-${param.id}'),
                  value: shownFace,
                  options: faces.contains(shownFace)
                      ? faces
                      : [shownFace, ...faces],
                  label: (f) => f,
                  face: mixed ? Text(_dash, style: t.body) : null,
                  onChanged: faces.isEmpty ? null : (f) => set(family, f),
                ),
              ),
              if (own) ...[
                const SizedBox(width: 2),
                _resetArrow(
                    t, 'copy-reset-${param.id}', () => _clear([param.id])),
              ],
            ],
          ),
        ),
      ),
    );
  }

  /// A footage control: a well with the picture and its name, which takes
  /// footage or a composition dropped from the Project panel, and how the
  /// picture is fitted beside it.
  Widget _wellRow(BuildContext context, _Draw draw, BridgeCopyControl c) {
    final t = draw.t;
    final fitRow = '${c.id}_fit';
    final fitParam = draw.params[fitRow];
    final item = _textOf(draw.shown(c.id));
    final fit = switch (draw.shown(fitRow)) {
      BridgeEffectValue_Choice(:final field0) => field0,
      _ => 0,
    };
    final fits = switch (fitParam?.kind) {
      BridgeParamKind_Choice(:final options) => options,
      _ => const <String>[],
    };
    final ownPicture = _differs(draw.params[c.id], draw.shown(c.id));
    final own = ownPicture || _differs(fitParam, draw.shown(fitRow));
    final mixed = _mixed([c.id, fitRow]);
    final held = _pictures[item];

    void set(UuidValue item, int fit) => _toEach((target) =>
        target.layer.setCopyValue(
            control: c.id,
            value: BridgeControlValue.picture(item: item, fit: fit),
            clip: target.clip));

    Widget face(bool lit) => Container(
          key: ValueKey<String>('copy-well-${c.id}'),
          height: 26,
          padding: const EdgeInsets.symmetric(horizontal: 3),
          decoration: BoxDecoration(
            color: t.surface0,
            borderRadius: BorderRadius.circular(t.tokens.wellRadius),
            border: Border.all(color: lit ? t.accent : t.hairline),
          ),
          // In a narrow panel the picture gives way to the name.
          child: LayoutBuilder(
            builder: (context, room) => Row(
              children: [
                if (room.maxWidth >= 72) ...[
                  ClipRRect(
                    borderRadius: BorderRadius.circular(t.tokens.wellRadius),
                    child: SizedBox(
                      width: 34,
                      height: 20,
                      child: mixed || held?.picture == null
                          ? ColoredBox(color: t.surface2)
                          : RawImage(image: held!.picture, fit: BoxFit.cover),
                    ),
                  ),
                  const SizedBox(width: 6),
                ],
                Expanded(
                  child: Text(
                    mixed ? _dash : (held?.name ?? ''),
                    style: (own || c.fed) && !mixed ? t.bodyPrimary : t.body,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ],
            ),
          ),
        );

    return draw.marked(
      c.id,
      _menued(
        context,
        c,
        rows: [c.id, fitRow],
        own: own,
        child: fxTwoColumnRow(
          context: context,
          grows: true,
          name: _label(t, c.label, own || c.fed),
          control: Padding(
            padding: const EdgeInsets.symmetric(vertical: 2),
            child: Row(
              children: [
                Expanded(
                  flex: 2,
                  child: DragTarget<FootageDragData>(
                    onWillAcceptWithDetails: (d) => d.data.footage.isNotEmpty,
                    onAcceptWithDetails: (d) =>
                        set(d.data.footage.first.internalid, fit),
                    builder: (context, footage, _) => DragTarget<CompDragData>(
                      onAcceptWithDetails: (d) =>
                          set(d.data.comp.internalid, fit),
                      builder: (context, comps, _) =>
                          face(footage.isNotEmpty || comps.isNotEmpty),
                    ),
                  ),
                ),
                if (fits.isNotEmpty) ...[
                  const SizedBox(width: 4),
                  Flexible(
                    child: BareDropdown<int>(
                      key: ValueKey<String>('copy-fit-${c.id}'),
                      value: fit.clamp(0, fits.length - 1).toInt(),
                      options: [for (var i = 0; i < fits.length; i++) i],
                      label: (i) => engineLabel(fits[i]),
                      // How a picture is fitted is the copy's to say once
                      // the picture is its own.
                      onChanged: ownPicture && !mixed
                          ? (i) => set(UuidValue.fromString(item), i)
                          : null,
                    ),
                  ),
                ],
                if (own) ...[
                  const SizedBox(width: 2),
                  _resetArrow(t, 'copy-reset-${c.id}',
                      () => _clear([c.id, fitRow])),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// The ticks at the foot of the card for a template with an intro or an
  /// outro: whether this copy plays each.
  Widget _playRow(BuildContext context, _Draw draw, BridgeParamInfo? intro,
      BridgeParamInfo? outro) {
    final t = draw.t;
    Widget tick(BridgeParamInfo p) {
      if (_mixed([p.id]) && !_revealed.contains(p.id)) {
        return _dashButton(t, 'copy-mixed-${p.id}',
            () => setState(() => _revealed.add(p.id)));
      }
      final on = switch (draw.shown(p.id)) {
        BridgeEffectValue_Bool(:final field0) => field0,
        _ => true,
      };
      return HouseCheckbox(
        key: ValueKey<String>('copy-${p.id}'),
        value: on,
        onChanged: (next) => _write(draw.id, p.id, BridgeEffectValue.bool(next)),
      );
    }

    Widget word(BridgeParamInfo p) => Text(
        t.propertyCase(engineLabel(p.label)),
        style: t.body,
        overflow: TextOverflow.ellipsis);

    final lead = intro ?? outro!;
    return fxTwoColumnRow(
      context: context,
      name: word(lead),
      control: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          tick(lead),
          if (intro != null && outro != null) ...[
            const SizedBox(width: 24),
            Flexible(child: word(outro)),
            const SizedBox(width: 8),
            tick(outro),
          ],
        ],
      ),
    );
  }
}

/// What every row of one build of the card is drawn from.
class _Draw {
  const _Draw({
    required this.t,
    required this.id,
    required this.params,
    required this.shown,
    required this.marked,
    required this.rowPath,
  });

  final LumitTheme t;

  /// The staged instance the rows belong to.
  final UuidValue id;
  final Map<String, BridgeParamInfo> params;

  /// A row's value, the one a drag is holding where there is one.
  final BridgeEffectValue? Function(String row) shown;

  /// [child] under the row's name in a shared project.
  final Widget Function(String row, Widget child) marked;

  /// Where the rows sit in the Timeline's selection, for a copy that is a
  /// layer. A clip's copy has no rows there.
  final String? rowPath;
}

/// A text control's box: as wide as the room it is given, and as tall as
/// its lines.
class _WordsBox extends StatelessWidget {
  const _WordsBox({
    super.key,
    required this.box,
    required this.node,
    required this.style,
    this.hint,
    required this.menuRows,
  });

  final TextEditingController box;
  final FocusNode node;
  final TextStyle style;
  final String? hint;
  final List<Widget> Function(VoidCallback close) menuRows;

  /// The well's own inset and edge, across and down.
  static const double _across = 14;
  static const double _down = 8;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
        builder: (context, room) => ListenableBuilder(
          listenable: box,
          builder: (context, _) {
            // Measured as the box will wrap it, so a long line that folds
            // counts as the lines it takes.
            final painter = TextPainter(
              text: TextSpan(
                  text: box.text.isEmpty ? ' ' : box.text, style: style),
              textDirection: Directionality.of(context),
              textScaler: MediaQuery.textScalerOf(context),
            )..layout(maxWidth: math.max(0.0, room.maxWidth - _across));
            final height = painter.height + _down;
            painter.dispose();
            return SizedBox(
              height: height,
              child: HouseTextField(
                controller: box,
                focusNode: node,
                width: room.maxWidth,
                multiline: true,
                style: style,
                hint: hint,
                menuRows: menuRows,
              ),
            );
          },
        ),
      );
}

/// The Row stepper on a copy's heading: the number of the data row the copy
/// follows, blank when it follows none, stepped with its arrows or typed.
/// Clearing the box stops following.
class _RowStepper extends StatefulWidget {
  const _RowStepper({
    super.key,
    required this.row,
    required this.count,
    required this.onFollow,
  });

  /// The row followed, counting from 0, and how many there are.
  final int? row;
  final int count;
  final ValueChanged<int?> onFollow;

  @override
  State<_RowStepper> createState() => _RowStepperState();
}

class _RowStepperState extends State<_RowStepper> {
  final TextEditingController _box = TextEditingController();
  final FocusNode _node = FocusNode(debugLabel: 'copy-row');

  @override
  void initState() {
    super.initState();
    _box.text = _shown;
    _node.addListener(() {
      if (!_node.hasFocus) _commit(_box.text);
    });
  }

  @override
  void didUpdateWidget(covariant _RowStepper old) {
    super.didUpdateWidget(old);
    if (!_node.hasFocus) _box.text = _shown;
  }

  @override
  void dispose() {
    _box.dispose();
    _node.dispose();
    super.dispose();
  }

  String get _shown => widget.row == null ? '' : '${widget.row! + 1}';

  void _commit(String text) {
    final typed = text.trim();
    final number = int.tryParse(typed);
    if (typed.isNotEmpty && number == null) {
      _box.text = _shown;
      return;
    }
    final row = number == null ? null : number.clamp(1, widget.count) - 1;
    _box.text = row == null ? '' : '${row + 1}';
    if (row != widget.row) widget.onFollow(row);
  }

  void _step(int by) {
    final from = widget.row ?? (by > 0 ? -1 : widget.count);
    final row = (from + by).clamp(0, widget.count - 1);
    if (row != widget.row) widget.onFollow(row);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final row = widget.row;
    Widget arrow(String key, String tip, String icon, bool enabled,
            VoidCallback onPressed) =>
        LumitTooltip(
          message: tip,
          child: HouseButton(
            key: ValueKey<String>(key),
            frameless: true,
            small: true,
            padding: const EdgeInsets.all(1),
            onPressed: enabled ? onPressed : null,
            child: LumitIcon(icon,
                size: 10, colour: enabled ? t.textMuted : t.textDisabled),
          ),
        );
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(t.propertyCase(l10n.templateCopyRow),
            style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
        const SizedBox(width: 2),
        arrow('copy-row-previous', l10n.templateCopyPreviousRow,
            LumitIcons.previousFrame, row == null || row > 0, () => _step(-1)),
        SizedBox(
          height: 18,
          child: HouseTextField(
            key: const ValueKey<String>('copy-row-box'),
            controller: _box,
            focusNode: _node,
            width: 34,
            padding: const EdgeInsets.symmetric(horizontal: 3, vertical: 1),
            textAlign: TextAlign.right,
            style: t.mono.copyWith(fontSize: 10, color: t.textPrimary),
            onSubmitted: _commit,
          ),
        ),
        arrow('copy-row-next', l10n.templateCopyNextRow, LumitIcons.nextFrame,
            row == null || row < widget.count - 1, () => _step(1)),
        const SizedBox(width: 6),
      ],
    );
  }
}
