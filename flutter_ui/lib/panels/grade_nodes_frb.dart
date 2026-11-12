// The Grade nodes panel: the current shot's grade drawn as nodes, left to
// right from the source picture to what the shot shows.
//
// Every node is an effect in the shot's own stack, in stack order, so this is
// another view of the stack Effect controls lists. A Correction set to
// Parallel or Layer shares the input of the node above it: the run is drawn
// as tiles stacked between a split and a mixer. A dashed line is a key wire,
// one Correction's key feeding a later effect's Matte.
//
// With thumbnails on, each tile shows what its node puts out: the shot drawn
// alone at the playhead, its grade stopping after that node.

import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' as dui;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/grade_groups.dart';
import 'package:lumit_flutter/src/rust/api/grade_shared.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/lumit_icon.dart';
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/clipboard.dart';
import '../state/dock.dart';
import '../state/grade_engine.dart';
import '../state/grade_target.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'grade_actions.dart';
import 'grade_clipboard.dart';
import 'grade_controls.dart' show GradeIconButton, GradeMenu, gradeInset;
import 'graph_panel.dart' show GraphNameField, portColour;
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'param_row_menu.dart' show menuGroups;
import 'timeline_extras_frb.dart' show DoubleTap, Segment, SegmentOption;

const double _tileW = 96, _plainH = 56;

/// How much taller a tile grows to show its node's picture.
const double _pictureH = 48;

/// The wire's run between two columns, and the room a split or mixer takes.
const double _gap = 36, _joint = 22;
const double _rowGap = 12;

class GradeNodesPanelFrb extends StatefulWidget {
  const GradeNodesPanelFrb({super.key});

  @override
  State<GradeNodesPanelFrb> createState() => _GradeNodesPanelFrbState();
}

/// One node as drawn: its effect and where its tile sits on the canvas.
typedef _Tile = ({BridgeEffectInstanceInfo info, Rect rect, int? number});

/// The canvas laid out: tiles, the joints of each run, and the two ends.
class _Layout {
  final List<_Tile> tiles = [];

  /// Each run's split and mixer points, with the mixer's kind.
  final List<(Offset split, Offset mixer, bool layer)> runs = [];

  /// Where the line leaves each column and where it comes in.
  final List<(Offset from, Offset to)> links = [];
  Offset source = Offset.zero, out = Offset.zero;
  Size size = Size.zero;

  /// Each tile's column and its place in that column, for a drop.
  final List<(int column, int row)> places = [];
  final List<(double left, double right)> columns = [];
}

class _GradeNodesPanelFrbState extends State<GradeNodesPanelFrb> {
  LumitUiState get ui => Provider.of<LumitUiState>(context, listen: false);
  LumitState get app => Provider.of<LumitState>(context, listen: false);

  final FocusNode _focus = FocusNode(debugLabel: 'grade-nodes');
  final DoubleTap _double = DoubleTap();

  /// The view: zoom and where the canvas's origin sits in the pane. Null
  /// fits the nodes to the pane, which a new shot goes back to.
  ({double zoom, Offset at})? _view;
  String? _viewOf;

  /// The node being dragged to a new place, and where the pointer is.
  UuidValue? _dragging;
  Offset _dragAt = Offset.zero;

  UuidValue? _renaming;

  /// The zoom the canvas was last drawn at, for a drag's steps.
  double _zoomShown = 1;

  bool get _thumbnails => ui.grade.nodeThumbnails.value;

  double get _tileH => _thumbnails ? _plainH + _pictureH : _plainH;

  // --- Node pictures -------------------------------------------------------

  /// Each node's picture and what it was made from; the old one stays up
  /// until its replacement is in.
  final Map<UuidValue, (String, dui.Image)> _pictures = {};

  /// The pictures wanted, the newest last, by node.
  final Map<UuidValue, (String, GradeShot, int)> _wanted = {};
  bool _making = false;
  Timer? _settle;

  /// The longest edge a node's picture is made at.
  static const int _pictureEdge = 192;

  /// What node [i] of [nodes] puts out depends on: the shot, the frame, and
  /// every node up to it, with the shot's own stack for a group's grade.
  String _pictureSign(GradeShot s, List<BridgeEffectInstanceInfo> nodes,
          int i, int frame) =>
      Object.hashAll([
        s.key,
        frame,
        if (_part != null)
          for (final e in shotStackOf(ui, s) ?? const []) ...[
            e.enabled,
            for (final v in e.values) '${v.id}=${v.value}',
          ],
        for (final e in nodes.take(i + 1)) ...[
          e.id,
          e.enabled,
          for (final v in e.values) '${v.id}=${v.value}',
        ],
      ]).toString();

  /// [e]'s picture, asked for when what it shows has changed. Made once
  /// the frame has settled, one at a time.
  dui.Image? _pictureOf(GradeShot s, List<BridgeEffectInstanceInfo> nodes,
      BridgeEffectInstanceInfo e) {
    final frame = ui.playheadFrame.value;
    final sign = _pictureSign(s, nodes, nodes.indexOf(e), frame);
    final held = _pictures[e.id];
    if (held?.$1 != sign && _wanted[e.id]?.$1 != sign) {
      _wanted.remove(e.id);
      _wanted[e.id] = (sign, s, frame);
      _settle?.cancel();
      _settle = Timer(const Duration(milliseconds: 200), _make);
    }
    return held?.$2;
  }

  Future<void> _make() async {
    if (_making) return;
    _making = true;
    while (mounted && _thumbnails && _wanted.isNotEmpty) {
      final id = _wanted.keys.last;
      final (sign, s, frame) = _wanted.remove(id)!;
      final CompositionReference? comp = ui.selectedComp;
      if (comp == null) break;
      dui.Image? image;
      try {
        final got = await comp.shotNodeThumbnail(
            layer: s.layer.internallayerId,
            clip: s.clip,
            node: id,
            frame: frame,
            maxEdge: _pictureEdge);
        if (got != null && got.width > 0) {
          final done = Completer<dui.Image>();
          dui.decodeImageFromPixels(got.rgba, got.width, got.height,
              dui.PixelFormat.rgba8888, done.complete);
          image = await done.future;
        }
      } catch (_) {
        image = null;
      }
      if (!mounted) {
        image?.dispose();
        break;
      }
      if (image == null) continue;
      final old = _pictures[id];
      _pictures[id] = (sign, image);
      setState(() {});
      WidgetsBinding.instance.addPostFrameCallback((_) => old?.$2.dispose());
    }
    _making = false;
  }

  /// Let go of the pictures, all of them or those of nodes not in [keep].
  void _dropPictures([Set<UuidValue>? keep]) {
    final gone = [
      for (final id in _pictures.keys)
        if (keep == null || !keep.contains(id)) id
    ];
    for (final id in gone) {
      final p = _pictures.remove(id);
      WidgetsBinding.instance.addPostFrameCallback((_) => p?.$2.dispose());
    }
    if (keep == null) _wanted.clear();
  }

  /// A moved playhead asks for new pictures once it rests.
  void _playheadMoved() {
    if (!_thumbnails) return;
    _settle?.cancel();
    _settle = Timer(const Duration(milliseconds: 300), () {
      if (mounted) setState(() {});
    });
  }

  late final ValueNotifier<int> _playhead = ui.playheadFrame;

  @override
  void initState() {
    super.initState();
    _playhead.addListener(_playheadMoved);
  }

  // --- What is shown -------------------------------------------------------

  GradeShot? get _shot => ui.gradeShot.value;

  bool get _usable {
    final s = _shot;
    return s != null && ui.selectedComp != null && shotStackOf(ui, s) != null;
  }

  (int?, String, UuidValue?)? _groupRead;

  /// The group the shot is in, read once per document revision.
  UuidValue? _group(GradeShot s) {
    final revision = ui.model.revision;
    final held = _groupRead;
    if (held != null && held.$1 == revision && held.$2 == s.key) return held.$3;
    UuidValue? read;
    try {
      read = s.layer.getGradeGroup(clip: s.clip);
    } catch (_) {
      read = null;
    }
    _groupRead = (revision, s.key, read);
    return read;
  }

  /// The group grade in view, or null for the shot's own. A shot that has
  /// left its group shows its own.
  BridgeGradePart? get _part {
    final s = _shot;
    if (s == null || _group(s) == null) return null;
    return switch (ui.grade.scope.value) {
      GradeScope.groupPre => BridgeGradePart.pre,
      GradeScope.groupPost => BridgeGradePart.post,
      _ => null,
    };
  }

  (int?, String, List<BridgeEffectInstanceInfo>)? _partRead;

  /// The nodes in view: the shot's stack, or one of its group's grades.
  List<BridgeEffectInstanceInfo> _nodes(GradeShot s) {
    final part = _part;
    final group = part == null ? null : _group(s);
    if (part == null || group == null) {
      return [
        for (final e in shotStackOf(ui, s) ?? const [])
          if (!e.audio) e,
      ];
    }
    final key = '${s.key}/$group/${part.name}';
    final revision = ui.model.revision;
    final held = _partRead;
    if (held != null && held.$1 == revision && held.$2 == key) return held.$3;
    List<BridgeEffectInstanceInfo> read;
    try {
      read = [
        for (final h in s.layer.getGradeGroupEffects(group: group, part_: part))
          h.getInfo()
      ];
    } catch (_) {
      read = const [];
    }
    _partRead = (revision, key, read);
    return read;
  }

  /// Fresh handles of the nodes in view, for an edit.
  List<BridgeEffectInstance> _handles(GradeShot s) {
    final part = _part;
    final group = part == null ? null : _group(s);
    return part == null || group == null
        ? shotHandles(s)
        : s.layer.getGradeGroupEffects(group: group, part_: part);
  }

  (int?, String, List<(UuidValue, UuidValue)>, Set<UuidValue>)? _keysRead;

  /// The key wires on a layer's stack: which Correction's key feeds which
  /// effect's Matte. A clip's stack has none.
  List<(UuidValue, UuidValue)> _keyWires(GradeShot s) => _keysOf(s).$1;

  /// The effects on a layer's stack with a Matte a key can be wired into.
  Set<UuidValue> _mattes(GradeShot s) => _keysOf(s).$2;

  (List<(UuidValue, UuidValue)>, Set<UuidValue>) _keysOf(GradeShot s) {
    if (s.clip != null || _part != null) return (const [], const {});
    final revision = ui.model.revision;
    final held = _keysRead;
    if (held != null && held.$1 == revision && held.$2 == s.key) {
      return (held.$3, held.$4);
    }
    final read = <(UuidValue, UuidValue)>[];
    final mattes = <UuidValue>{};
    try {
      final graph = s.layer.getGraph();
      for (final e in graph.wiring.edges) {
        if (e.from case BridgeOutputRef_EffectKey(effect: final from)) {
          if (e.to case BridgeInputRef_Matte(effect: final to)) {
            read.add((from, to));
          }
        }
      }
      for (final n in graph.nodes) {
        if (n.node case BridgeNodeRef_Effect(:final field0)) {
          if (n.inputs.any((p) => p.id == 'matte')) mattes.add(field0);
        }
      }
    } catch (_) {
      // The layer went; no wires to draw.
    }
    _keysRead = (revision, s.key, read, mattes);
    return (read, mattes);
  }

  // --- Key wires -------------------------------------------------------------

  /// A key wire being drawn: from which node's key, the node it fed when an
  /// end was picked up, and where the pointer is on the canvas.
  ({UuidValue from, UuidValue? was, Offset at})? _wiring;

  static bool _into(BridgeGraphEdge e, Set<UuidValue> ids) => switch (e.to) {
        BridgeInputRef_Matte(:final effect) => ids.contains(effect),
        _ => false,
      };

  /// [from]'s key wired into [to]'s Matte in place of what fed it, and the
  /// wire into [drop] taken away, as one step called [name]. With [from]
  /// null, the wires go and nothing takes their place.
  void _rewire(UuidValue? from, UuidValue? to,
      {UuidValue? drop, required String name}) {
    final s = _shot;
    if (s == null || s.clip != null) return;
    final gone = {if (to != null) to, if (drop != null) drop};
    asOneUndoStep(app.project, () {
      try {
        final w = s.layer.getGraph().wiring;
        s.layer.setGraph(
          drivers: s.layer.getGraphDrivers(),
          wiring: BridgeGraphWiring(
            edges: [
              for (final e in w.edges)
                if (!_into(e, gone)) e,
              if (from != null && to != null)
                BridgeGraphEdge(
                    from: BridgeOutputRef.effectKey(effect: from),
                    to: BridgeInputRef.matte(effect: to)),
            ],
            layout: w.layout,
            exposed: w.exposed,
            groups: w.groups,
            outUnwired: w.outUnwired,
          ),
        );
      } catch (_) {
        // A wire the graph refuses: the document is as it was.
      }
    }, name: name);
    _tell();
  }

  static bool _inverted(BridgeEffectInstanceInfo e) => e.values.any((v) =>
      v.id == 'matte_invert' &&
      v.value is BridgeEffectValue_Bool &&
      (v.value as BridgeEffectValue_Bool).field0);

  /// [to] takes the opposite of the key it is fed, or the key itself again.
  void _invertKey(BridgeEffectInstanceInfo to) {
    final s = _shot;
    if (s == null) return;
    asOneUndoStep(app.project, () {
      try {
        final handles = shotHandles(s);
        for (final h in handles) {
          if (h.id() == to.id) {
            h.setValue(
                id: 'matte_invert',
                value: BridgeEffectValue.bool(!_inverted(to)));
          }
        }
        s.layer.setEffects(effects: handles);
      } catch (_) {
        // A locked layer refuses.
      }
    }, name: 'Invert key');
    _tell();
  }

  /// The tile a wire dropped at canvas point [p] lands on: one after [from]
  /// in the stack with a Matte to take it. Null for none.
  UuidValue? _wireTarget(_Layout l, Offset p, UuidValue from,
      List<BridgeEffectInstanceInfo> nodes, Set<UuidValue> mattes) {
    final at = nodes.indexWhere((e) => e.id == from);
    for (final tile in l.tiles) {
      final id = tile.info.id;
      if (!tile.rect.inflate(10).contains(p)) continue;
      if (nodes.indexWhere((e) => e.id == id) > at && mattes.contains(id)) {
        return id;
      }
    }
    return null;
  }

  void _wireEnd(_Layout l, List<BridgeEffectInstanceInfo> nodes,
      Set<UuidValue> mattes) {
    final w = _wiring;
    setState(() => _wiring = null);
    if (w == null) return;
    final to = _wireTarget(l, w.at, w.from, nodes, mattes);
    if (to == w.was) return;
    if (to == null) {
      // An end picked up and let go over nothing: the wire goes.
      if (w.was != null) _rewire(null, null, drop: w.was, name: 'Remove key wire');
      return;
    }
    _rewire(w.from, to, drop: w.was, name: 'Wire key');
  }

  List<Widget> _wireMenu(
          VoidCallback close, UuidValue from, BridgeEffectInstanceInfo to) =>
      [
        _row('grade-nodes-key-invert', l10n.gradeKeyInvert, close,
            () => _invertKey(to),
            ticked: _inverted(to)),
        _row('grade-nodes-key-remove', l10n.gradeKeyRemove, close,
            () => _rewire(null, null, drop: to.id, name: 'Remove key wire'),
            ticked: false),
      ];

  static int _choice(BridgeEffectInstanceInfo e, String id) {
    for (final v in e.values) {
      if (v.id == id) {
        if (v.value case BridgeEffectValue_Choice(:final field0)) return field0;
      }
    }
    return 0;
  }

  /// Parallel 1, Layer 2, Serial 0. Only a Correction joins a run.
  static int _combine(BridgeEffectInstanceInfo e) =>
      e.name == correctionName ? _choice(e, 'combine') : 0;

  /// The node in view the Grade panel edits.
  UuidValue? _current(List<BridgeEffectInstanceInfo> nodes) {
    final want = ui.gradeNode.value;
    if (nodes.any((e) => e.id == want)) return want;
    return nodes.where((e) => e.name == correctionName).lastOrNull?.id;
  }

  // --- Layout --------------------------------------------------------------

  _Layout _layout(List<BridgeEffectInstanceInfo> nodes) {
    final l = _Layout();
    final numbers = {
      for (final (i, e) in gradeNumberedOf(nodes).indexed) e.id: i + 1,
    };
    // Columns: a node, and the nodes that join its run.
    final columns = <List<BridgeEffectInstanceInfo>>[];
    for (final (i, e) in nodes.indexed) {
      if (i > 0 && _combine(e) != 0) {
        columns.last.add(e);
      } else {
        columns.add([e]);
      }
    }
    final tallest = columns.fold<int>(1, (m, c) => math.max(m, c.length));
    final height = tallest * _tileH + (tallest - 1) * _rowGap;
    final mid = height / 2;
    var x = 0.0;
    l.source = Offset(x, mid);
    var last = l.source;
    x += _gap;
    for (final (ci, column) in columns.indexed) {
      final run = column.length > 1;
      final left = x;
      if (run) x += _joint;
      final tall = column.length * _tileH + (column.length - 1) * _rowGap;
      final top = mid - tall / 2;
      for (final (ri, e) in column.indexed) {
        final rect = Rect.fromLTWH(
            x, top + ri * (_tileH + _rowGap), _tileW, _tileH);
        l.tiles.add((info: e, rect: rect, number: numbers[e.id]));
        l.places.add((ci, ri));
      }
      x += _tileW;
      if (run) {
        final split = Offset(left, mid);
        final mixer = Offset(x + _joint, mid);
        l.runs.add((split, mixer, column.skip(1).any((e) => _combine(e) == 2)));
        l.links.add((last, split));
        last = mixer;
        x += _joint;
      } else {
        l.links.add((last, Offset(left, mid)));
        last = Offset(x, mid);
      }
      l.columns.add((left, x));
      x += _gap;
    }
    // With no nodes the wire still spans a tile, the plus where it would go.
    if (columns.isEmpty) x += _tileW;
    l.out = Offset(x, mid);
    l.links.add((last, l.out));
    l.size = Size(x, height);
    return l;
  }

  /// The zoom and origin that fit [size] in [pane].
  ({double zoom, Offset at}) _fit(Size size, Size pane) {
    const pad = 16.0;
    final zoom = math.min(
        1.0,
        math.min((pane.width - pad * 2) / math.max(1, size.width),
            (pane.height - pad * 2) / math.max(1, size.height)));
    final z = zoom.clamp(0.3, 1.0);
    return (
      zoom: z,
      at: Offset((pane.width - size.width * z) / 2,
          (pane.height - size.height * z) / 2),
    );
  }

  // --- Edits ---------------------------------------------------------------

  void _tell() {
    app.notifyDocumentChanged();
    ui.model.refresh();
  }

  void _select(UuidValue id) {
    ui.grade.setNode(id);
    _focus.requestFocus();
  }

  void _add(GradeNodeKind kind, [UuidValue? after]) {
    final s = _shot;
    if (s == null) return;
    final part = _part;
    final group = part == null ? null : _group(s);
    if (part == null || group == null) {
      addGradeNode(app, ui, kind, after: after);
      return;
    }
    // A group's grade takes serial nodes, at its end.
    try {
      s.layer.addGradeGroupEffect(
          group: group, part_: part, name: correctionName);
    } catch (_) {
      return;
    }
    _tell();
  }

  /// [run] on the handle of node [id], as one step named [name].
  void _onNode(UuidValue id, String name,
      void Function(GradeShot s, BridgeEffectInstance h) run) {
    final s = _shot;
    if (s == null) return;
    asOneUndoStep(app.project, () {
      try {
        for (final h in _handles(s)) {
          if (h.id() == id) run(s, h);
        }
      } catch (_) {
        // A locked layer refuses; the document is as it was.
      }
    }, name: name);
    _tell();
  }

  void _bypass(BridgeEffectInstanceInfo e) => _onNode(e.id, 'Bypass node',
      (s, h) => s.layer.setEffectEnabled(effect: h, enabled: !e.enabled));

  void _delete(UuidValue id) {
    _onNode(id, 'Delete node', (s, h) => s.layer.removeEffect(effect: h));
    if (ui.gradeNode.value == id) ui.grade.setNode(null);
  }

  void _reset(UuidValue id) {
    final s = _shot;
    if (s == null) return;
    asOneUndoStep(app.project, () {
      try {
        if (_part == null) {
          resetNodesOn(s, {id});
        } else {
          final handles = _handles(s);
          resetNodesOnHandles(handles, {id});
          s.layer.setEffects(effects: handles);
        }
      } catch (_) {
        // A locked layer refuses.
      }
    }, name: 'Reset node');
    _tell();
  }

  /// A shared node's own effect back to its defaults, in every shot.
  void _resetShared(BridgeSharedNode shared) {
    final s = _shot;
    final effect = shared.effect;
    if (s == null || effect == null) return;
    asOneUndoStep(app.project, () {
      try {
        final handles = s.layer
            .getGradeGroupEffects(group: shared.id, part_: BridgeGradePart.pre);
        resetNodesOnHandles(handles, {effect});
        s.layer.setEffects(effects: handles);
      } catch (_) {
        // A locked layer refuses.
      }
    }, name: 'Reset node');
    _tell();
  }

  void _rename(UuidValue id, String name) {
    setState(() => _renaming = null);
    final s = _shot;
    if (s == null) return;
    // A shared node's name is its own, the same in every shot.
    if (sharedNodeOf(ui, id) case final shared? when name.trim().isNotEmpty) {
      try {
        app.project?.renameGradeGroup(group: shared.id, name: name.trim());
      } catch (_) {
        // Gone under the field.
      }
      _tell();
      return;
    }
    try {
      final handles = _handles(s);
      for (final h in handles) {
        if (h.id() == id) h.setCustomName(name: name);
      }
      s.layer.setEffects(effects: handles, clip: _part == null ? s.clip : null);
    } catch (_) {
      // The stack moved under the field.
    }
    _tell();
  }

  /// Node [id] to just before node [before], or to the end with none. The
  /// stack may hold effects the panel leaves out, so the place is found in
  /// the stack itself.
  void _move(UuidValue id, UuidValue? before) {
    final s = _shot;
    if (s == null) return;
    final rest = [
      for (final h in _handles(s))
        if (h.id() != id) h.id()
    ];
    final index = before == null ? rest.length : rest.indexOf(before);
    if (index < 0) return;
    _onNode(id, 'Move node',
        (s, h) => s.layer.reorderEffect(effect: h, newIndex: index));
  }

  void _copy(UuidValue id) {
    final s = _shot;
    if (s == null || _part != null) return;
    try {
      ui.copyEffectsToClipboard(s.layer.copyEffects(effects: [id]));
    } catch (_) {
      // Gone; the clipboard keeps what it had.
    }
  }

  bool get _canPaste =>
      _part == null &&
      ui.clipboard.kind == ClipboardKind.effects &&
      ui.clipboard.text != null;

  /// The clipboard's effects as nodes after [after], or at the end.
  void _paste(UuidValue? after) {
    final s = _shot;
    final text = ui.clipboard.text;
    if (s == null || text == null || !_canPaste) return;
    asOneUndoStep(app.project, () {
      try {
        final before = {for (final h in shotHandles(s)) h.id()};
        s.clip == null
            ? s.layer.pasteEffects(text: text, atFrame: ui.playheadFrame.value)
            : s.layer.loadPreset(text: text, clip: s.clip);
        final handles = shotHandles(s);
        var at = after == null
            ? handles.length
            : handles.indexWhere((h) => h.id() == after) + 1;
        for (final h in handles) {
          if (before.contains(h.id())) continue;
          s.layer.reorderEffect(effect: h, newIndex: at++);
        }
      } catch (_) {
        // Not effects this stack can take.
      }
    }, name: 'Paste node');
    _tell();
  }

  void _reveal(UuidValue id) {
    final s = _shot;
    if (s == null) return;
    ui.grade.setNode(id);
    if (s.clip == null) ui.setEffectSelection(s.layer, [id]);
    setPanelVisible(ui.split, Panel.effectControls, true);
    activatePanelTab(ui.split, Panel.effectControls);
    ui.activePane.value = Panel.effectControls.pane();
    ui.workspace.touch();
  }

  // --- Menus ---------------------------------------------------------------

  MenuRow _row(String key, String label, VoidCallback close, VoidCallback run,
          {bool enabled = true, bool? ticked, String? action}) =>
      MenuRow(
        key: ValueKey<String>(key),
        enabled: enabled,
        onPressed: () {
          close();
          run();
        },
        child:
            MenuLabel(label, ticked: ticked, enabled: enabled, action: action),
      );

  List<Widget> _addRows(VoidCallback close, UuidValue? after) {
    final group = _part != null;
    return [
      for (final kind in GradeNodeKind.values)
        if (!group || kind == GradeNodeKind.serial)
          _row(
              'grade-nodes-add-${kind.name}',
              switch (kind) {
                GradeNodeKind.serial => l10n.gradeAddSerial,
                GradeNodeKind.parallel => l10n.gradeAddParallel,
                GradeNodeKind.layer => l10n.gradeAddLayer,
                GradeNodeKind.outside => l10n.gradeAddOutside,
              },
              close,
              () => _add(kind, after),
              enabled: group || canAddGradeNode(ui, kind),
              ticked: false,
              action: 'grade.node.${kind.name}'),
      _row('grade-nodes-add-warper', l10n.gradeAddWarper, close,
          () => addGradeEffectNode(app, ui, colourWarperName, after: after),
          enabled: canAddGradeEffect(ui), ticked: false),
      if (!group)
        if (_sharedAddRow(close, after) case final row?) row,
    ];
  }

  /// Add shared node: every shared node by its name, one the shot already
  /// uses greyed out. Left out while there are none.
  Widget? _sharedAddRow(VoidCallback close, UuidValue? after) {
    final s = _shot;
    final shared = sharedGradeNodes(ui);
    if (s == null || shared.isEmpty) return null;
    bool uses(BridgeSharedNode n) => n.users.any(
        (u) => u.layer == s.layer.internallayerId && u.clip == s.clip);
    return SubmenuRow(
      key: const ValueKey<String>('grade-nodes-add-shared'),
      closeParent: close,
      submenu: (dismiss) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final n in shared)
                _row('grade-nodes-add-shared-${n.id}', n.name, dismiss,
                    () => _addShared(n.id, after),
                    enabled: !uses(n), ticked: false),
            ],
          ),
        ),
      ),
      child: MenuLabel(l10n.gradeAddShared),
    );
  }

  /// Shared node [id] in this shot after [after], made current. One step.
  void _addShared(UuidValue id, UuidValue? after) {
    final s = _shot;
    if (s == null) return;
    try {
      final made = s.layer.addSharedGradeNode(
          clip: s.clip, node: id, after: after ?? _current(_nodes(s)));
      ui.grade.setNode(made);
    } catch (_) {
      return;
    }
    _tell();
  }

  /// Node [id] becomes a shared node, used by every other picked shot too.
  /// This shot keeps it where it was, now as the shared node.
  void _share(UuidValue id) {
    final s = _shot;
    final comp = ui.selectedComp;
    if (s == null || comp == null) return;
    final others = [
      for (final o in gradeTargets(ui))
        if (!o.same(s))
          BridgeShotId(
              comp: comp.internalid,
              layer: o.layer.internallayerId,
              clip: o.clip),
    ];
    UuidValue made;
    try {
      made = s.layer.shareGradeNode(clip: s.clip, node: id, shots: others);
    } catch (_) {
      return;
    }
    _tell();
    // The node in this shot is the shared node's reference now.
    final refs = sharedGradeNodes(ui)
            .where((n) => n.id == made)
            .firstOrNull
            ?.references ??
        const [];
    final here = _nodes(s).where((e) => refs.contains(e.id)).firstOrNull;
    if (here != null) ui.grade.setNode(here.id);
  }

  /// Every shot using [shared] gets its own copy where the reference stood;
  /// this shot's copy stays current.
  void _unshare(BridgeEffectInstanceInfo e, BridgeSharedNode shared) {
    final s = _shot;
    if (s == null) return;
    final at = _nodes(s).indexWhere((n) => n.id == e.id);
    try {
      app.project?.unshareGradeNode(node: shared.id);
    } catch (_) {
      return;
    }
    _tell();
    final nodes = _nodes(s);
    if (at >= 0 && at < nodes.length) ui.grade.setNode(nodes[at].id);
  }

  List<Widget> _nodeMenu(VoidCallback close, BridgeEffectInstanceInfo e) {
    final shot = _shot;
    final own = _part == null;
    final shared = isSharedReference(e) ? sharedNodeOf(ui, e.id) : null;
    // What a CDL export writes: the node, or a shared node's own effect.
    final (cdlNode, cdlName) = shared == null
        ? (e.id, e.name)
        : (shared.effect, shared.matchName ?? '');
    return menuGroups([
      _addRows(close, e.id),
      [
        _row('grade-nodes-bypass', l10n.gradeBypassNode, close,
            () => _bypass(e),
            ticked: !e.enabled),
        _row('grade-nodes-reset', l10n.gradeResetNode, close,
            () => shared == null ? _reset(e.id) : _resetShared(shared),
            ticked: false),
        _row('grade-nodes-rename', l10n.renameEllipsis, close,
            () => setState(() => _renaming = e.id),
            ticked: false),
        _row('grade-nodes-delete', l10n.gradeDeleteNode, close,
            () => _delete(e.id),
            ticked: false),
      ],
      [
        if (own && shared == null && gradeEffectNames.contains(e.name))
          _row('grade-nodes-share', l10n.gradeShareNode, close,
              () => _share(e.id),
              ticked: false),
        if (shared != null)
          _row('grade-nodes-unshare', l10n.gradeUnshareNode, close,
              () => _unshare(e, shared),
              ticked: false),
      ],
      [
        if (own)
          _row('grade-nodes-cdl-import', l10n.gradeImportCdl, close,
              () => importCdlFile(context, app, ui, after: e.id),
              ticked: false),
        if (cdlNode != null && cdlExportable(cdlName))
          _row('grade-nodes-cdl-export', l10n.gradeExportCdl, close,
              () => exportCdlFile(app, ui, cdlNode,
                  name: shared?.name ?? e.customName),
              ticked: false),
      ],
      [
        if (own)
          _row('grade-nodes-copy', l10n.gradeCopyNode, close, () => _copy(e.id),
              ticked: false),
        if (own)
          _row('grade-nodes-paste', l10n.gradePasteNode, close,
              () => _paste(e.id),
              enabled: _canPaste, ticked: false),
        if (own && shot != null)
          _row('grade-nodes-look', l10n.gradeSaveLook, close,
              () => saveShotLook(context, app, ui, shot),
              enabled: hasGrade(ui), ticked: false),
        if (own)
          _row('grade-nodes-reveal', l10n.gradeRevealInEffectControls, close,
              () => _reveal(e.id),
              ticked: false),
      ],
    ]);
  }

  List<Widget> _groundMenu(VoidCallback close) {
    if (!_usable) return const [];
    final own = _part == null;
    return menuGroups([
      _addRows(close, null),
      [
        if (own)
          _row('grade-nodes-paste-end', l10n.gradePasteNode, close,
              () => _paste(null),
              enabled: _canPaste, ticked: false),
      ],
      [
        if (own && hasGrade(ui))
          _row('grade-nodes-reset-grade', l10n.gradeResetAll, close,
              () => resetGrade(app, ui),
              ticked: false),
        if (own && hasGrade(ui))
          _row('grade-nodes-bypass-grade', l10n.gradeBypassAll, close,
              () => bypassGrade(app, ui),
              ticked: gradeBypassed(ui)),
      ],
    ]);
  }

  // --- Keys and pointer ----------------------------------------------------

  KeyEventResult _key(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent || !node.hasPrimaryFocus) {
      return KeyEventResult.ignored;
    }
    final s = _shot;
    if (s == null || !_usable) return KeyEventResult.ignored;
    final nodes = _nodes(s);
    final current = _current(nodes);
    final at = nodes.indexWhere((e) => e.id == current);
    final key = event.logicalKey;
    if (current != null &&
        (key == LogicalKeyboardKey.delete ||
            key == LogicalKeyboardKey.backspace)) {
      _delete(current);
      return KeyEventResult.handled;
    }
    if (current != null &&
        (key == LogicalKeyboardKey.enter || key == LogicalKeyboardKey.f2)) {
      setState(() => _renaming = current);
      return KeyEventResult.handled;
    }
    // Left and right walk the nodes, round from either end.
    if (nodes.isNotEmpty &&
        (key == LogicalKeyboardKey.arrowLeft ||
            key == LogicalKeyboardKey.arrowRight)) {
      final step = key == LogicalKeyboardKey.arrowLeft ? -1 : 1;
      ui.grade.setNode(nodes[(at + step) % nodes.length].id);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  void _wheel(PointerSignalEvent event, ({double zoom, Offset at}) view) {
    if (event is! PointerScrollEvent) return;
    final zoom =
        (view.zoom * (event.scrollDelta.dy > 0 ? 0.9 : 1.1)).clamp(0.3, 2.0);
    // About the pointer: the spot under it stays under it.
    final p = event.localPosition;
    final at = p - (p - view.at) * (zoom / view.zoom);
    setState(() => _view = (zoom: zoom, at: at));
  }

  /// The stack place a drop at canvas point [p] lands on.
  int _dropIndex(_Layout l, Offset p, UuidValue dragged) {
    var column = l.columns.length;
    for (final (i, (_, right)) in l.columns.indexed) {
      if (p.dx < right) {
        column = i;
        break;
      }
    }
    var index = 0;
    for (final (i, tile) in l.tiles.indexed) {
      if (tile.info.id == dragged) continue;
      final (c, _) = l.places[i];
      final before = c < column ||
          (c == column &&
              (p.dx > tile.rect.right ||
                  (p.dx >= tile.rect.left && p.dy > tile.rect.center.dy)));
      if (before) index++;
    }
    return index;
  }

  // --- Drawing -------------------------------------------------------------

  @override
  void dispose() {
    _playhead.removeListener(_playheadMoved);
    _settle?.cancel();
    _dropPictures();
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final ui = this.ui;
    final grade = ui.grade;
    return ListenableBuilder(
      listenable: Listenable.merge([
        ui.model,
        grade.shot,
        grade.node,
        grade.scope,
        grade.nodeThumbnails,
      ]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        final shot = _shot;
        final usable = _usable;
        if (_viewOf != shot?.key) {
          _viewOf = shot?.key;
          _view = null;
          _renaming = null;
        }
        final nodes =
            usable ? _nodes(shot!) : const <BridgeEffectInstanceInfo>[];
        if (!_thumbnails) {
          if (_pictures.isNotEmpty || _wanted.isNotEmpty) _dropPictures();
        } else if (_pictures.length > nodes.length) {
          _dropPictures({for (final e in nodes) e.id});
        }
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _header(t, shot, usable),
            Expanded(
              child: GradeMenu(
                rows: _groundMenu,
                child: Focus(
                  focusNode: _focus,
                  onKeyEvent: _key,
                  child: Container(
                    color: t.surface0,
                    child: LayoutBuilder(
                      builder: (context, box) =>
                          _canvas(t, shot, nodes, usable, box.biggest),
                    ),
                  ),
                ),
              ),
            ),
          ],
        );
      },
    );
  }

  Widget _header(LumitTheme t, GradeShot? shot, bool usable) {
    final group = usable ? _group(shot!) : null;
    final scope = ui.grade.scope.value;
    return Container(
      height: 26,
      color: t.surface1,
      padding: const EdgeInsets.only(left: gradeInset, right: 4),
      child: Row(
        children: [
          Expanded(
            child: group == null
                ? Text(usable ? gradeShotName(ui, shot!) : '',
                    style: t.small.copyWith(color: t.textMuted),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis)
                : SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    child: Segment(children: [
                      for (final s in const [
                        GradeScope.shot,
                        GradeScope.groupPre,
                        GradeScope.groupPost,
                      ])
                        SegmentOption(
                          key: ValueKey<String>('grade-nodes-scope-${s.name}'),
                          active: s == scope,
                          onTap: () => ui.grade.scope.value = s,
                          child: Text(
                              switch (s) {
                                GradeScope.groupPre => l10n.gradeScopeGroupPre,
                                GradeScope.groupPost =>
                                  l10n.gradeScopeGroupPost,
                                _ => l10n.gradeScopeShot,
                              },
                              style: t.small.copyWith(
                                  color: s == scope
                                      ? t.textPrimary
                                      : t.textSecondary)),
                        ),
                    ]),
                  ),
          ),
          GradeIconButton(
            key: const ValueKey('grade-nodes-thumbnails'),
            glyph: LumitIcons.still,
            tip: l10n.gradeNodeThumbnails,
            active: _thumbnails,
            onPressed: () => ui.grade.nodeThumbnails.value = !_thumbnails,
          ),
          GradeMenu(
            rows: (close) => usable ? _addRows(close, null) : const [],
            child: GradeIconButton(
              key: const ValueKey('grade-nodes-add'),
              glyph: LumitIcons.addEffect,
              tip: l10n.gradeAddSerial,
              onPressed: usable &&
                      (_part != null ||
                          canAddGradeNode(ui, GradeNodeKind.serial))
                  ? () => _add(GradeNodeKind.serial)
                  : null,
            ),
          ),
        ],
      ),
    );
  }

  Widget _canvas(LumitTheme t, GradeShot? shot,
      List<BridgeEffectInstanceInfo> nodes, bool usable, Size pane) {
    final l = _layout(nodes);
    final view = _view ?? _fit(l.size, pane);
    _zoomShown = view.zoom;
    final current = _current(nodes);
    final wires = usable ? _keyWires(shot!) : const <(UuidValue, UuidValue)>[];
    final mattes = usable ? _mattes(shot!) : const <UuidValue>{};
    final rects = {for (final tile in l.tiles) tile.info.id: tile.rect};
    // Key dots: a Correction's key out, and the Matte in of any node that
    // takes one. A clip carries no wires, so its dots are drawn dimmed.
    final wireable = usable && shot!.clip == null && _part == null;
    final dotted = usable && _part == null;
    final outs = [
      if (dotted)
        for (final tile in l.tiles)
          if (tile.info.name == correctionName) tile,
    ];
    final ins = [
      if (dotted)
        for (final tile in l.tiles)
          if (shot!.clip != null
              ? tile.info.name == correctionName
              : mattes.contains(tile.info.id))
            tile,
    ];
    final wiring = _wiring;
    final dragged = _dragging;
    int? drop;
    if (dragged != null) drop = _dropIndex(l, _dragAt, dragged);
    return Listener(
      onPointerDown: (_) => _focus.requestFocus(),
      onPointerSignal: (e) => _wheel(e, view),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        // Dragging the ground pans; a double-click fits the nodes again.
        onPanUpdate: (d) =>
            setState(() => _view = (zoom: view.zoom, at: view.at + d.delta)),
        onTapUp: (_) {
          if (_double.tap()) setState(() => _view = null);
        },
        child: ClipRect(
          child: Stack(
            children: [
              Positioned(
                left: view.at.dx,
                top: view.at.dy,
                child: Transform.scale(
                  scale: view.zoom,
                  alignment: Alignment.topLeft,
                  child: SizedBox(
                    width: l.size.width,
                    height: l.size.height + _tileH,
                    child: Stack(
                      clipBehavior: Clip.none,
                      children: [
                        Positioned.fill(
                          child: CustomPaint(
                            painter: _WirePainter(
                              t: t,
                              layout: l,
                              keys: [
                                for (final (a, b) in wires)
                                  if (rects[a] case final ra?)
                                    if (rects[b] case final rb?)
                                      if (wiring?.was != b) (ra, rb),
                              ],
                              enabled: usable,
                              drop: drop == null || dragged == null
                                  ? null
                                  : _dropMark(l, drop, dragged),
                            ),
                          ),
                        ),
                        // A right-click on a key wire offers Invert and
                        // Remove. Under the tiles, so a tile wins.
                        if (wireable)
                          for (final (a, b) in wires)
                            if (rects[a] case final ra?)
                              if (rects[b] case final rb?)
                                if (nodes.where((e) => e.id == b).firstOrNull
                                    case final to?)
                                  Positioned.fill(
                                    child: GestureDetector(
                                      key: ValueKey<String>(
                                          'grade-nodes-wire-$a-$b'),
                                      onSecondaryTapDown: (d) =>
                                          GradeMenu.show(
                                              context,
                                              d.globalPosition,
                                              (close) =>
                                                  _wireMenu(close, a, to)),
                                      child: CustomPaint(
                                          painter: _WireHit(_keyPath(ra, rb))),
                                    ),
                                  ),
                        for (final tile in l.tiles)
                          Positioned.fromRect(
                            rect: tile.rect,
                            child: _tile(t, tile, tile.info.id == current,
                                nodes.indexWhere((e) => e.id == tile.info.id),
                                l),
                          ),
                        if (nodes.isEmpty)
                          Positioned(
                            left: l.size.width / 2 - 10,
                            top: l.size.height / 2 - 10,
                            child: _plus(t, usable),
                          ),
                        // The key dots over the tiles' edges, and a wire on
                        // its way.
                        Positioned.fill(
                          child: IgnorePointer(
                            child: CustomPaint(
                              painter: _DotPainter(
                                t: t,
                                outs: [for (final x in outs) _keyOut(x.rect)],
                                ins: [for (final x in ins) _keyIn(x.rect)],
                                enabled: wireable,
                                live: wiring == null ||
                                        rects[wiring.from] == null
                                    ? null
                                    : (_keyOut(rects[wiring.from]!), wiring.at),
                              ),
                            ),
                          ),
                        ),
                        // Drag from a key out to a later node to wire it;
                        // drag a wire's end to another node, or off to
                        // remove it.
                        if (wireable)
                          for (final x in outs)
                            _dotHandle('grade-nodes-key-${x.info.id}',
                                _keyOut(x.rect), (at) => (from: x.info.id, was: null, at: at),
                                l, nodes, mattes),
                        if (wireable)
                          for (final (a, b) in wires)
                            if (rects[b] case final rb?)
                              _dotHandle('grade-nodes-matte-$b', _keyIn(rb),
                                  (at) => (from: a, was: b, at: at),
                                  l, nodes, mattes),
                      ],
                    ),
                  ),
                ),
              ),
              if (dragged != null && rects[dragged] != null)
                Positioned(
                  left: view.at.dx + (_dragAt.dx - _tileW / 2) * view.zoom,
                  top: view.at.dy + (_dragAt.dy - _tileH / 2) * view.zoom,
                  child: IgnorePointer(
                    child: Opacity(
                      opacity: 0.6,
                      child: Transform.scale(
                        scale: view.zoom,
                        alignment: Alignment.topLeft,
                        child: SizedBox(
                          width: _tileW,
                          height: _tileH,
                          child: _face(
                              t,
                              l.tiles.firstWhere((x) => x.info.id == dragged),
                              true),
                        ),
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

  /// A grab spot over a key dot at canvas point [dot]. A drag from it draws
  /// the wire [start] begins.
  Widget _dotHandle(
      String key,
      Offset dot,
      ({UuidValue from, UuidValue? was, Offset at}) Function(Offset at) start,
      _Layout l,
      List<BridgeEffectInstanceInfo> nodes,
      Set<UuidValue> mattes) {
    const r = 8.0;
    final corner = dot - const Offset(r, r);
    return Positioned(
      left: corner.dx,
      top: corner.dy,
      width: r * 2,
      height: r * 2,
      child: MouseRegion(
        cursor: SystemMouseCursors.precise,
        child: GestureDetector(
          key: ValueKey<String>(key),
          behavior: HitTestBehavior.opaque,
          onPanStart: (d) =>
              setState(() => _wiring = start(corner + d.localPosition)),
          onPanUpdate: (d) {
            final w = _wiring;
            if (w == null) return;
            setState(() => _wiring =
                (from: w.from, was: w.was, at: corner + d.localPosition));
          },
          onPanEnd: (_) => _wireEnd(l, nodes, mattes),
          onPanCancel: () => setState(() => _wiring = null),
        ),
      ),
    );
  }

  /// Where a drop at stack place [index] would go, as a short upright line.
  (Offset, Offset)? _dropMark(_Layout l, int index, UuidValue dragged) {
    final others = [
      for (final tile in l.tiles)
        if (tile.info.id != dragged) tile.rect
    ];
    if (others.isEmpty) return null;
    final rect = index < others.length ? others[index] : others.last;
    final x = index < others.length
        ? rect.left - _gap / 2
        : rect.right + _gap / 2;
    return (Offset(x, rect.top), Offset(x, rect.bottom));
  }

  Widget _plus(LumitTheme t, bool usable) => GestureDetector(
        key: const ValueKey('grade-nodes-empty-add'),
        onTap: usable ? () => _add(GradeNodeKind.serial) : null,
        child: HoverMark(
          builder: (context, lit) => Container(
            width: 20,
            height: 20,
            alignment: Alignment.center,
            decoration: BoxDecoration(
              color: Color.lerp(t.surface1, t.surface2, lit),
              shape: BoxShape.circle,
              border: Border.all(color: t.hairlineStrong),
            ),
            child: Text('+',
                style: t.small.copyWith(
                    color: usable ? t.textSecondary : t.textDisabled)),
          ),
        ),
      );

  Widget _tile(LumitTheme t, _Tile tile, bool current, int index, _Layout l) {
    final e = tile.info;
    Offset? start;
    return GradeMenu(
      key: ValueKey<String>('grade-nodes-tile-${e.id}'),
      rows: (close) {
        ui.grade.setNode(e.id);
        return _nodeMenu(close, e);
      },
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTapUp: (_) {
          _select(e.id);
          if (_double.tap()) setState(() => _renaming = e.id);
        },
        onPanStart: (d) {
          start = d.localPosition;
          _select(e.id);
          setState(() {
            _dragging = e.id;
            _dragAt = tile.rect.center;
          });
        },
        onPanUpdate: (d) => setState(() => _dragAt += d.delta / _zoomShown),
        onPanEnd: (_) {
          final moved = _dragging;
          final to = moved == null ? null : _dropIndex(l, _dragAt, moved);
          setState(() => _dragging = null);
          if (start == null || moved == null || to == null || to == index) {
            return;
          }
          final others = [
            for (final x in l.tiles)
              if (x.info.id != moved) x.info.id
          ];
          _move(moved, to < others.length ? others[to] : null);
        },
        onPanCancel: () => setState(() => _dragging = null),
        child: Opacity(
          opacity: _dragging == e.id ? 0.3 : 1,
          child: _face(t, tile, current),
        ),
      ),
    );
  }

  /// The glyphs for what a Correction uses: curves, a qualifier, windows.
  /// A LUT is its own node.
  static List<String> _uses(BridgeEffectInstanceInfo e) {
    if (e.name == 'lut') return [LumitIcons.colourSpace];
    if (e.name != correctionName) return const [];
    var curves = false, windows = false;
    for (final v in e.values) {
      if (v.value case BridgeEffectValue_Curve(:final field0)) {
        final flat = v.id.startsWith('curve_') ? null : 0.5;
        if (field0.any((p) =>
            p.length >= 2 && ((flat ?? p[0]) - p[1]).abs() > 1e-3)) {
          curves = true;
        }
      }
      if (v.id.startsWith('win') && v.id.endsWith('_shape')) {
        if (v.value case BridgeEffectValue_Choice(:final field0)
            when field0 != 0) {
          windows = true;
        }
      }
    }
    return [
      if (curves) LumitIcons.bezier,
      if (_choice(e, 'qual_mode') != 0) LumitIcons.eyedropper,
      if (windows) LumitIcons.ellipse,
    ];
  }

  Widget _face(LumitTheme t, _Tile tile, bool current) {
    final e = tile.info;
    final off = !e.enabled;
    final shared = isSharedReference(e) ? sharedNodeOf(ui, e.id) : null;
    final label =
        shared?.name ?? e.customName ?? engineLabel(effectLabelOf(e.name));
    final renaming = _renaming == e.id;
    return CustomPaint(
      foregroundPainter: off ? _SlashPainter(t.textDisabled) : null,
      child: Container(
        padding: const EdgeInsets.fromLTRB(6, 4, 6, 4),
        decoration: BoxDecoration(
          color: t.surface1,
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          border: Border.all(
              color: current ? t.accent : t.hairlineStrong,
              width: current ? 1.5 : 1),
        ),
        child: Opacity(
          opacity: off ? 0.45 : 1,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  if (tile.number case final n?)
                    Text('$n',
                        style: t.small.copyWith(
                            color: current ? t.textPrimary : t.textSecondary)),
                  const Spacer(),
                  if (shared != null)
                    LumitIcon(LumitIcons.link,
                        size: 11, colour: t.textSecondary),
                  for (final g in _uses(e))
                    Padding(
                      padding: const EdgeInsets.only(left: 2),
                      child: LumitIcon(g, size: 10, colour: t.textMuted),
                    ),
                ],
              ),
              if (_thumbnails)
                Expanded(child: _picture(t, e))
              else
                const Spacer(),
              if (renaming)
                SizedBox(
                  height: 20,
                  child: GraphNameField(
                    key: ValueKey<String>('grade-nodes-rename-${e.id}'),
                    initial: shared?.name ?? e.customName ?? '',
                    onDone: (name) => _rename(e.id, name),
                    onCancel: () => setState(() => _renaming = null),
                  ),
                )
              else
                Text(label,
                    style: t.small.copyWith(
                        color: current ? t.textPrimary : t.textSecondary),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis),
            ],
          ),
        ),
      ),
    );
  }

  /// What the node puts out, between its number and its name.
  Widget _picture(LumitTheme t, BridgeEffectInstanceInfo e) {
    final s = _shot;
    final image = s == null || !_usable ? null : _pictureOf(s, _nodes(s), e);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(t.tokens.actionRadius),
        child: ColoredBox(
          color: t.surface2,
          child: image == null
              ? const SizedBox.expand()
              : RawImage(
                  key: ValueKey<String>('grade-nodes-picture-${e.id}'),
                  image: image,
                  fit: BoxFit.cover,
                  width: double.infinity,
                  height: double.infinity),
        ),
      ),
    );
  }

  static final Map<String, String> _labels = {
    for (final e in listEffects()) e.name: e.label,
  };
  static String effectLabelOf(String name) => _labels[name] ?? name;
}

/// The picture's line through the nodes, the runs' splits and mixers, the
/// key wires dashed under the tiles, and where a dragged node would land.
class _WirePainter extends CustomPainter {
  final LumitTheme t;
  final _Layout layout;
  final List<(Rect, Rect)> keys;
  final bool enabled;
  final (Offset, Offset)? drop;

  _WirePainter(
      {required this.t,
      required this.layout,
      required this.keys,
      required this.enabled,
      required this.drop});

  @override
  void paint(Canvas canvas, Size size) {
    final colour = enabled
        ? portColour(t, BridgePortType.image)
        : t.textDisabled;
    final line = Paint()
      ..color = colour
      ..strokeWidth = 1.5
      ..style = PaintingStyle.stroke;
    final dot = Paint()..color = colour;
    void curve(Offset a, Offset b) {
      final mid = (a.dx + b.dx) / 2;
      canvas.drawPath(
          Path()
            ..moveTo(a.dx, a.dy)
            ..cubicTo(mid, a.dy, mid, b.dy, b.dx, b.dy),
          line);
    }

    for (final (a, b) in layout.links) {
      curve(a, b);
    }
    // Each run: the split fans out to its tiles and the mixer gathers them.
    for (final (split, mixer, isLayer) in layout.runs) {
      for (final tile in layout.tiles) {
        final r = tile.rect;
        if (r.left > split.dx && r.right < mixer.dx) {
          curve(split, r.centerLeft);
          curve(r.centerRight, mixer);
        }
      }
      canvas.drawCircle(split, 3, dot);
      canvas.drawCircle(mixer, 8, Paint()..color = t.surface2);
      canvas.drawCircle(mixer, 8, line);
      final g = Paint()
        ..color = colour
        ..strokeWidth = 1.5;
      void bar(Offset a, Offset b) =>
          canvas.drawLine(mixer + a, mixer + b, g);
      if (isLayer) {
        // Layer: two stacked bars.
        bar(const Offset(-4, -2), const Offset(4, -2));
        bar(const Offset(-4, 2), const Offset(4, 2));
      } else {
        // Parallel: a plus, the changes added.
        bar(const Offset(-4, 0), const Offset(4, 0));
        bar(const Offset(0, -4), const Offset(0, 4));
      }
    }
    canvas.drawCircle(layout.source, 5, dot);
    canvas.drawCircle(layout.out, 5, dot);
    // Key wires: from one tile's key out to another's Matte in, dipping
    // below both.
    final key = Paint()
      ..color = portColour(t, BridgePortType.matte)
      ..strokeWidth = 1.2
      ..style = PaintingStyle.stroke;
    for (final (a, b) in keys) {
      _dashed(canvas, _keyPath(a, b), key);
    }
    if (drop case (final a, final b)?) {
      canvas.drawLine(
          a,
          b,
          Paint()
            ..color = t.accent
            ..strokeWidth = 2);
    }
  }

  @override
  bool shouldRepaint(_WirePainter old) => true;
}

/// How far a key dot sits from the middle of a tile's lower edge: the key
/// out to the right, the Matte in to the left.
const double _dotSpread = 14;

Offset _keyOut(Rect r) => r.bottomCenter + const Offset(_dotSpread, 0);
Offset _keyIn(Rect r) => r.bottomCenter - const Offset(_dotSpread, 0);

/// A key wire from tile [a]'s key out to tile [b]'s Matte in.
Path _keyPath(Rect a, Rect b) {
  final from = _keyOut(a), to = _keyIn(b);
  final depth = math.max(from.dy, to.dy) + 20;
  return Path()
    ..moveTo(from.dx, from.dy)
    ..cubicTo(from.dx, depth, to.dx, depth, to.dx, to.dy);
}

void _dashed(Canvas canvas, Path path, Paint paint) {
  for (final m in path.computeMetrics()) {
    for (var d = 0.0; d < m.length; d += 7) {
      canvas.drawPath(m.extractPath(d, math.min(d + 4, m.length)), paint);
    }
  }
}

/// The key dots and the wire being drawn, over the tiles.
class _DotPainter extends CustomPainter {
  final LumitTheme t;
  final List<Offset> outs, ins;
  final bool enabled;
  final (Offset, Offset)? live;

  _DotPainter(
      {required this.t,
      required this.outs,
      required this.ins,
      required this.enabled,
      required this.live});

  @override
  void paint(Canvas canvas, Size size) {
    final colour =
        enabled ? portColour(t, BridgePortType.matte) : t.textDisabled;
    final fill = Paint()..color = t.surface1;
    final ring = Paint()
      ..color = colour
      ..strokeWidth = 1.2
      ..style = PaintingStyle.stroke;
    for (final p in outs) {
      canvas.drawCircle(p, 3.5, Paint()..color = colour);
    }
    for (final p in ins) {
      canvas.drawCircle(p, 3.5, fill);
      canvas.drawCircle(p, 3.5, ring);
    }
    if (live case (final a, final b)?) {
      final depth = math.max(a.dy, b.dy) + 20;
      _dashed(
          canvas,
          Path()
            ..moveTo(a.dx, a.dy)
            ..cubicTo(a.dx, depth, b.dx, depth, b.dx, b.dy),
          ring);
    }
  }

  @override
  bool shouldRepaint(_DotPainter old) => true;
}

/// Paints nothing: says whether a point is on a key wire, for its menu.
class _WireHit extends CustomPainter {
  final Path path;
  _WireHit(this.path);

  @override
  void paint(Canvas canvas, Size size) {}

  @override
  bool? hitTest(Offset position) {
    for (final m in path.computeMetrics()) {
      for (var d = 0.0; d <= m.length; d += 3) {
        final p = m.getTangentForOffset(d)?.position;
        if (p != null && (p - position).distance < 5) return true;
      }
    }
    return false;
  }

  @override
  bool shouldRepaint(_WireHit old) => old.path != path;
}

/// A bypassed node's slash, corner to corner.
class _SlashPainter extends CustomPainter {
  final Color colour;
  _SlashPainter(this.colour);

  @override
  void paint(Canvas canvas, Size size) => canvas.drawLine(
      Offset(size.width - 6, 6),
      Offset(6, size.height - 6),
      Paint()
        ..color = colour
        ..strokeWidth = 1.2);

  @override
  bool shouldRepaint(_SlashPainter old) => old.colour != colour;
}
