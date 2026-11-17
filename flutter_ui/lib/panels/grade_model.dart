// The Grade panel's view of the document: the current node of the current
// shot, its values at the playhead, and every write the panel makes.
//
// A write lands on the shot shown and on every other picked shot of the same
// kind, in one undo step: picked layers when a layer is the shot, picked Cut
// clips when a clip is. A drag previews the shot shown alone, as the renderer
// previews one layer; the others take the value on release.

part of 'grade_panel_frb.dart';

/// The rows Reset node leaves alone: what the node is, not what it does.
const Set<String> _structure = {'space', 'combine', 'part'};

mixin _GradeModel on State<GradePanelFrb> {
  final EffectStackEditor _editor = EffectStackEditor();
  static const GradeEngine _engine = GradeEngine();

  LumitUiState get ui => Provider.of<LumitUiState>(context, listen: false);
  LumitState get app => Provider.of<LumitState>(context, listen: false);

  /// Each effect's declaration, read once per effect: the defaults and ranges
  /// every control takes come from the engine, never from a table here.
  static final Map<String, Map<String, BridgeParamInfo>> _schemas = {};
  static Map<String, BridgeParamInfo> _schemaOf(String name) =>
      _schemas.putIfAbsent(name, () {
        try {
          return {for (final p in listParameters(effect: name)) p.id: p};
        } catch (_) {
          return const {};
        }
      });

  /// The room a page has under the page bar, or null while the pages stack
  /// in one scroll and size by width alone.
  Size? _room;

  // --- The shot and its stack ------------------------------------------------

  GradeShot? get _shot => ui.gradeShot.value;

  BridgeLayerInfo? _layerInfo(GradeShot? s) =>
      s == null ? null : ui.model.byId(s.layer.internallayerId)?.info;

  /// Whether the shot draws a picture a grade can sit on.
  bool _gradeable(GradeShot? s) {
    final info = _layerInfo(s);
    return info != null && gradeable(info.kind);
  }

  /// A clip's stack on the comp's clock, read once per document revision so
  /// a value at the playhead reads and keys as a layer's does.
  final Map<String, (int?, List<BridgeEffectInstanceInfo>)> _clipStacks = {};

  List<BridgeEffectInstanceInfo> _stackOf(GradeShot? s) {
    if (s == null) return const [];
    final info = _layerInfo(s);
    if (info == null) return const [];
    if (_scopeOn(s) case final scope?) return _groupStack(s, scope);
    final clip = s.clip;
    if (clip == null) return info.effects;
    final revision = ui.model.revision;
    final held = _clipStacks[s.key];
    if (held != null && held.$1 == revision) return held.$2;
    List<BridgeEffectInstanceInfo> read;
    try {
      read = [
        for (final e in s.layer.getClipEffects(clip: clip, onCompClock: true))
          e.getInfo()
      ];
    } catch (_) {
      // The clip went under the panel.
      read = const [];
    }
    _clipStacks[s.key] = (revision, read);
    return read;
  }

  List<BridgeEffectInstanceInfo> get _stack => _stackOf(_shot);

  /// The group grade shown on [s] while the scope is a group's: only ever
  /// on the shot shown. Null for its own stack.
  ({UuidValue group, BridgeGradePart part})? _scopeOn(GradeShot s) {
    final shown = _shot;
    return shown != null && s.same(shown) ? gradeScopeOf(ui) : null;
  }

  /// A group grade's nodes, read once per document revision.
  (int?, String, List<BridgeEffectInstanceInfo>)? _groupRead;

  List<BridgeEffectInstanceInfo> _groupStack(
      GradeShot s, ({UuidValue group, BridgeGradePart part}) scope) {
    final key = '${scope.group}/${scope.part.name}';
    final revision = ui.model.revision;
    final held = _groupRead;
    if (held != null && held.$1 == revision && held.$2 == key) return held.$3;
    List<BridgeEffectInstanceInfo> read;
    try {
      read = [
        for (final h in s.layer
            .getGradeGroupEffects(group: scope.group, part_: scope.part))
          h.getInfo()
      ];
    } catch (_) {
      read = const [];
    }
    _groupRead = (revision, key, read);
    return read;
  }

  /// The live handles of [s]'s stack, or of the group grade in view, for an
  /// edit.
  List<BridgeEffectInstance> _handles(GradeShot s) {
    if (_scopeOn(s) case final scope?) {
      return s.layer
          .getGradeGroupEffects(group: scope.group, part_: scope.part);
    }
    return shotHandles(s);
  }

  /// [handles] from [_handles] written back as one op.
  void _commitHandles(GradeShot s, List<BridgeEffectInstance> handles) =>
      s.layer.setEffects(
          effects: handles, clip: _scopeOn(s) == null ? s.clip : null);

  /// While set, rows read and write the shot's first effect of this name in
  /// place of the current node: the Look page's LUT.
  String? _aside;

  /// [run] with rows aimed at [aside]'s effect, or the current node for
  /// null. A control built inside keeps its aim for its own callbacks.
  T _within<T>(String? aside, T Function() run) {
    final was = _aside;
    _aside = aside;
    try {
      return run();
    } finally {
      _aside = was;
    }
  }

  /// The node [ui.gradeNode] names when it is in [stack], else the last
  /// Correction, else none: the first touch then adds one. A shared node's
  /// reference stands for the shared node's own effect.
  BridgeEffectInstanceInfo? _nodeIn(List<BridgeEffectInstanceInfo> stack) {
    if (_aside case final name?) {
      return stack.where((e) => e.name == name).firstOrNull;
    }
    final want = ui.gradeNode.value;
    if (want != null) {
      for (final e in stack) {
        if (e.id != want) continue;
        return isSharedReference(e) ? _sharedEffect : e;
      }
    }
    return stack.where((e) => e.name == correctionName).lastOrNull;
  }

  /// The shared node picked on the shot shown, while its own grade is in
  /// view.
  BridgeSharedNode? get _shared {
    final shot = _shot;
    if (shot == null || _scopeOn(shot) != null) return null;
    return sharedNodeOf(ui, ui.gradeNode.value);
  }

  /// The shot's reference to the shared node picked, for what acts on the
  /// node's place in this shot: bypass, delete.
  BridgeEffectInstanceInfo? get _sharedRef {
    if (_shared == null) return null;
    final want = ui.gradeNode.value;
    return _stack.where((e) => e.id == want).firstOrNull;
  }

  /// The shared node's list, for the editor and the preview.
  ({UuidValue group, BridgeGradePart part})? get _sharedList =>
      switch (_shared) {
        final s? => (group: s.id, part: BridgeGradePart.pre),
        null => null,
      };

  (int?, UuidValue, BridgeEffectInstanceInfo?)? _sharedRead;

  /// The shared node's one effect, read once per document revision.
  BridgeEffectInstanceInfo? get _sharedEffect {
    final shot = _shot;
    final shared = _shared;
    if (shot == null || shared == null) return null;
    final revision = ui.model.revision;
    final held = _sharedRead;
    if (held != null && held.$1 == revision && held.$2 == shared.id) {
      return held.$3;
    }
    BridgeEffectInstanceInfo? read;
    try {
      read = shot.layer
          .getGradeGroupEffects(group: shared.id, part_: BridgeGradePart.pre)
          .firstOrNull
          ?.getInfo();
    } catch (_) {
      read = null;
    }
    _sharedRead = (revision, shared.id, read);
    return read;
  }

  BridgeEffectInstanceInfo? get _node => _nodeIn(_stack);

  /// The effect the current node is, a new Correction when there is none.
  String get _nodeName => _aside ?? _node?.name ?? correctionName;
  bool get _onCorrection => _nodeName == correctionName;
  Map<String, BridgeParamInfo> get _schema => _schemaOf(_nodeName);

  /// The whole grade on [s]: every grade node in its stack.
  Set<UuidValue> _gradeIds([GradeShot? s]) =>
      {for (final e in gradeNodesOf(_stackOf(s ?? _shot))) e.id};

  bool get _anyPresent => _gradeIds().isNotEmpty;

  bool get _gradeOff {
    final nodes = gradeNodesOf(_stack);
    return nodes.isNotEmpty && nodes.every((e) => !e.enabled);
  }

  // --- Values at the playhead ------------------------------------------------

  BridgeEffectValue? _value(String id, [BridgeEffectInstanceInfo? on]) {
    final node = on ?? _node;
    // A drag in hand shows its own value, so a field beside it moves too.
    if (on == null && node != null) {
      if (_editor.stagedValue(node.id, id) case final staged?) return staged;
    }
    for (final v in node?.values ?? const <BridgeParamValue>[]) {
      if (v.id == id) return v.value;
    }
    return null;
  }

  BridgeParamKind? _kind(String id) => _schema[id]?.kind;

  String _label(String id) => engineLabel(_schema[id]?.label ?? id);

  double _defaultFloat(String id) => switch (_kind(id)) {
        BridgeParamKind_Float(:final default_) => default_,
        BridgeParamKind_Angle(:final default_) => default_,
        BridgeParamKind_Slider(:final default_) => default_,
        _ => 0,
      };

  (double, double) _range(String id) => switch (_kind(id)) {
        BridgeParamKind_Float(:final sliderMin, :final sliderMax) => (
            sliderMin,
            sliderMax
          ),
        BridgeParamKind_Angle() => (-180, 180),
        BridgeParamKind_Slider(:final min, :final max) => (min, max),
        _ => (0, 1),
      };

  /// The bounds typing keeps to: the hard ones, or open where the effect
  /// declares none, as an Effect controls field reads them.
  (double, double) _hardRange(String id) => switch (_kind(id)) {
        BridgeParamKind_Float(:final hardMin, :final hardMax) => (
            hardMin ?? -1000000,
            hardMax ?? 1000000
          ),
        BridgeParamKind_Angle() => (-1000000, 1000000),
        // A closed slider's range is both its travel and its bounds.
        BridgeParamKind_Slider(:final min, :final max) => (min, max),
        _ => (0, 1),
      };

  /// A number as it reads at the playhead. An expression reads as its
  /// default and its control stands down: there is no number to drag.
  double _float(String id, [BridgeEffectInstanceInfo? on]) {
    final comp = ui.selectedComp;
    final v = _value(id, on);
    if (comp == null || v is! BridgeEffectValue_Float) return _defaultFloat(id);
    return scalarValueAt(v.field0, comp, ui.playheadFrame.value) ??
        _defaultFloat(id);
  }

  bool _keyed(String id) => switch (_value(id)) {
        BridgeEffectValue_Float(:final field0) =>
          field0 is BridgeScalar_Keyframed,
        BridgeEffectValue_Colour(field0: final c) =>
          [c.r, c.g, c.b].any((s) => s is BridgeScalar_Keyframed),
        _ => false,
      };

  bool _locked(String id) {
    final v = _value(id);
    return v is BridgeEffectValue_Float && v.field0 is BridgeScalar_Expression;
  }

  static bool _near(double a, double b) => (a - b).abs() < 1e-6;

  List<double> _defaultColour(String id) => switch (_kind(id)) {
        BridgeParamKind_Colour(:final default_) => [
            default_[0],
            default_[1],
            default_[2]
          ],
        _ => const [0, 0, 0],
      };

  (double, double) _colourRange(String id) => switch (_kind(id)) {
        BridgeParamKind_Colour(:final min, :final max) => (min, max),
        _ => (0.0, 1.0),
      };

  List<double> _colour(String id) {
    final comp = ui.selectedComp;
    final v = _value(id);
    if (comp == null || v is! BridgeEffectValue_Colour) {
      return _defaultColour(id);
    }
    final time = timeOfFrame(comp, ui.playheadFrame.value);
    final c = v.field0;
    return [
      sampledScalar(c.r, time),
      sampledScalar(c.g, time),
      sampledScalar(c.b, time)
    ];
  }

  int _choice(String id) => switch (_value(id)) {
        BridgeEffectValue_Choice(:final field0) => field0,
        _ => switch (_kind(id)) {
            BridgeParamKind_Choice(:final default_) => default_,
            _ => 0,
          },
      };

  bool _bool(String id) => switch (_value(id)) {
        BridgeEffectValue_Bool(:final field0) => field0,
        _ => switch (_kind(id)) {
            BridgeParamKind_Bool(:final default_) => default_,
            _ => false,
          },
      };

  /// A curve's points; [_curveRest] when the node has none yet.
  List<List<double>> _curve(String id) {
    final v = _value(id);
    return v is BridgeEffectValue_Curve
        ? curvePointsOf(v.field0)
        : _curveRest(id);
  }

  /// Where a curve rests: the diagonal for a tone curve, the flat middle for
  /// a hue curve, as the effect declares it.
  List<List<double>> _curveRest(String id) {
    final kind = _kind(id);
    final d = kind == null ? null : defaultEffectValue(kind);
    return d is BridgeEffectValue_Curve ? curvePointsOf(d.field0) : curveIdentity;
  }

  /// Whether row [id] has left its default on the current node.
  bool _changed(String id) {
    if (_node == null) return false;
    return switch (_kind(id)) {
      BridgeParamKind_Float() ||
      BridgeParamKind_Angle() ||
      BridgeParamKind_Slider() =>
        !_near(_float(id), _defaultFloat(id)),
      BridgeParamKind_Colour() => () {
          final now = _colour(id), was = _defaultColour(id);
          return [for (var i = 0; i < 3; i++) !_near(now[i], was[i])]
              .any((x) => x);
        }(),
      BridgeParamKind_Choice(:final default_) => _choice(id) != default_,
      BridgeParamKind_Bool(:final default_) => _bool(id) != default_,
      // A point list: a moved mesh point or a kept sample.
      BridgeParamKind_Shape() => switch (_value(id)) {
          BridgeEffectValue_Shape(:final field0) =>
            field0.any((p) => p.any((x) => x.abs() > 1e-6)),
          _ => false,
        },
      _ => _value(id) is BridgeEffectValue_Curve &&
          !curveIs(_curve(id), _curveRest(id)),
    };
  }

  bool _anyChanged(Iterable<String> ids) => ids.any(_changed);

  bool get _nodeChanged => _node != null &&
      _schema.keys.where((id) => !_structure.contains(id)).any(_changed);

  // --- Undo groups -------------------------------------------------------------

  /// Undo groups opened by a gesture and not yet closed.
  int _touches = 0;

  /// Whether the open gesture added a node, which a cancel takes back.
  bool _touchAdded = false;

  /// The project a touch opened its group on, so the group can be closed
  /// from `dispose`, where nothing may be looked up.
  ProjectReference? _touched;

  /// The open gesture's newest preview, asked for again after a document
  /// change: the Viewer asks for the document then, and the newer ask would
  /// stand in front of the preview, the scope with it, until the next move.
  VoidCallback? _restage;

  /// A gesture begins: everything it writes, the node it may add included,
  /// is one undo step until [_touchEnd].
  void _touchStart() {
    final project = app.project;
    if (project == null) return;
    project.beginUndoGroup();
    project.nameUndoGroup(name: 'Grade');
    _touched = project;
    if (_touches == 0) _touchAdded = false;
    _touches++;
  }

  void _touchEnd() {
    if (_touches == 0) return;
    _touches--;
    if (_touches == 0) _restage = null;
    _touched?.endUndoGroup();
  }

  /// A gesture that ends with nothing written: the preview it staged goes,
  /// and the Viewer is asked for the document again. A node it added to
  /// preview on is undone with its step, so nothing is left in the history.
  void _touchCancel() {
    _editor.clear();
    _touchEnd();
    if (_touches == 0 && _touchAdded) {
      _touchAdded = false;
      _touched?.undo();
      _tell();
    }
    ui.requestFrame();
  }

  void _tell() {
    app.notifyDocumentChanged();
    ui.model.refresh();
  }

  // --- Where a write lands -----------------------------------------------------

  /// The shots an edit lands on: the one shown first, then every other picked
  /// shot of its kind that can wear a grade.
  List<GradeShot> get _targets {
    final shown = _shot;
    if (shown == null) return const [];
    // A group's grade is one grade, whichever shot shows it, and a shared
    // node one node.
    if (_scopeOn(shown) != null || _shared != null) return [shown];
    final comp = ui.selectedComp?.internalid;
    return [
      shown,
      ...shown.clip != null
          ? [
              for (final c in ui.cutClips.value)
                if (c.clip != shown.clip && c.layer.internalcompId == comp)
                  GradeShot(c.layer, c.clip),
            ]
          : [
              for (final layer in ui.selectedLayers.value)
                if (layer.internallayerId != shown.layer.internallayerId &&
                    _gradeable(GradeShot(layer)))
                  GradeShot(layer),
            ],
    ];
  }

  /// The node on [s] an edit of the current node lands on: the current node
  /// itself on the shot shown; on another shot the node of the same effect in
  /// the same place among its kind. A Correction is added where a shot has
  /// none, inside the gesture so it is part of the step.
  UuidValue? _ensureOn(GradeShot s) {
    final shown = _shot;
    final node = _node;
    final stack = _stackOf(s);
    final name = _nodeName;
    if (shown != null && s.same(shown)) {
      if (node != null) return node.id;
    } else {
      final mine = node == null
          ? 0
          : _stack.where((e) => e.name == name).toList().indexOf(node);
      final theirs = stack.where((e) => e.name == name).toList();
      if (mine >= 0 && mine < theirs.length) return theirs[mine].id;
      if (mine > 0 || (name != correctionName && _aside == null)) return null;
    }
    // The first touch: the node after the shot's grade, else first. A
    // group's grade takes it at its end.
    final after = gradeNodesOf(stack).lastOrNull?.id;
    final scope = _scopeOn(s);
    final made = scope != null
        ? _addToGroup(s, scope, name)
        : name == correctionName
        ? _engine.addNode(s, after, GradeNodeKind.serial)
        : _addEffectAfter(s, name, after);
    if (made == null) return null;
    if (_touches > 0) _touchAdded = true;
    if (shown != null && s.same(shown) && _aside == null) {
      ui.grade.setNode(made);
    }
    ui.model.refresh();
    return made;
  }

  /// Effect [name] added to the end of a group's grade.
  UuidValue? _addToGroup(GradeShot s,
      ({UuidValue group, BridgeGradePart part}) scope, String name) {
    try {
      s.layer.addGradeGroupEffect(
          group: scope.group, part_: scope.part, name: name);
      return s.layer
          .getGradeGroupEffects(group: scope.group, part_: scope.part)
          .lastOrNull
          ?.id();
    } catch (_) {
      return null;
    }
  }

  /// Effect [name] added to [s] right after [after], or first.
  UuidValue? _addEffectAfter(GradeShot s, String name, UuidValue? after) {
    try {
      final before = {for (final e in _handles(s)) e.id()};
      s.clip == null
          ? s.layer.addEffect(name: name)
          : s.layer.addClipEffect(clip: s.clip!, name: name);
      final handles = _handles(s);
      final made = handles.where((h) => !before.contains(h.id())).firstOrNull;
      if (made == null) return null;
      final at =
          after == null ? 0 : handles.indexWhere((h) => h.id() == after) + 1;
      s.layer.reorderEffect(effect: made, newIndex: at);
      return made.id();
    } catch (_) {
      return null;
    }
  }

  /// The layer's name, or for a clip the name of what it plays.
  String _shotName(GradeShot shot) {
    final info = _layerInfo(shot);
    if (info == null) return '';
    for (final c in info.clips) {
      if (c.id == shot.clip) return c.sourceName;
    }
    return info.name;
  }

  /// Point the editor at the stack [s]'s ids live in.
  void _aim(GradeShot s) {
    final clip = s.clip;
    final shown = _shot;
    final list =
        _scopeOn(s) ?? (shown != null && s.same(shown) ? _sharedList : null);
    _editor.groupStack = list == null
        ? null
        : () =>
            s.layer.getGradeGroupEffects(group: list.group, part_: list.part);
    // The engine stages a group's list or a shared node's by its ids, so a
    // drag on one previews as a layer's own rows do.
    _editor.gradeList = list != null;
    _editor.clipStack = clip == null || list != null
        ? null
        : () => s.layer.getClipEffects(clip: clip, onCompClock: true);
  }

  void _liveMany(Map<String, BridgeEffectValue> values) {
    final shot = _shot;
    final comp = ui.selectedComp;
    if (shot == null || comp == null) return;
    final id = _ensureOn(shot);
    if (id == null) return;
    _aim(shot);
    final frame = ui.playheadFrame.value;
    final scale = ui.viewerScale;
    _restage = () {
      if (!mounted) return;
      for (final e in values.entries) {
        _editor.live(comp, shot.layer, id, e.key, e.value,
            frame: frame, scale: scale);
      }
    };
    _restage!();
  }

  void _live(String param, BridgeEffectValue value) =>
      _liveMany({param: value});

  /// [values] makes each row's value from what the node it lands on holds, so
  /// a keyed row keeps its keys. [all] false writes the shot shown alone, for
  /// a key or an expression, which belong to the one row they were set on.
  void _writeMany(
      Map<String, BridgeEffectValue Function(BridgeEffectInstanceInfo? on)>
          values,
      {bool all = true}) {
    final shown = _shot;
    for (final s in all ? _targets : [if (shown != null) shown]) {
      final id = _ensureOn(s);
      if (id == null) continue;
      final on = _stackOf(s).where((e) => e.id == id).firstOrNull;
      _aim(s);
      _editor.writeAll(s.layer, id, {
        for (final e in values.entries) e.key: e.value(on),
      });
    }
    _touchEnd();
    _tell();
  }

  void _write(String param,
          BridgeEffectValue Function(BridgeEffectInstanceInfo? on) valueFor,
          {bool all = true}) =>
      _writeMany({param: valueFor}, all: all);

  /// A one-step write of rows that hold no keys: a switch, a choice, a curve.
  void _set(Map<String, BridgeEffectValue> values) {
    if (_touches == 0) _touchStart();
    _writeMany({for (final e in values.entries) e.key: (_) => e.value});
  }

  BridgeScalar _scalarFor(String id, double v, [BridgeEffectInstanceInfo? on]) {
    final comp = ui.selectedComp;
    final was = _value(id, on);
    final (lo, hi) = switch (_kind(id)) {
      BridgeParamKind_Float(:final hardMin, :final hardMax) => (
          hardMin ?? double.negativeInfinity,
          hardMax ?? double.infinity
        ),
      BridgeParamKind_Slider(:final min, :final max) => (min, max),
      _ => (double.negativeInfinity, double.infinity),
    };
    final clamped = v.clamp(lo, hi).toDouble();
    if (comp != null && was is BridgeEffectValue_Float) {
      return scalarWithValueAt(
          was.field0, clamped, comp, ui.playheadFrame.value);
    }
    return BridgeScalar.static_(clamped);
  }

  BridgeEffectValue _floatFor(String id, double v,
          [BridgeEffectInstanceInfo? on]) =>
      BridgeEffectValue.float(_scalarFor(id, v, on));

  void _liveFloat(String id, double v) => _live(id, _floatFor(id, v));

  void _writeFloat(String id, double v) =>
      _write(id, (on) => _floatFor(id, v, on));

  BridgeEffectValue _colourFor(String id, List<double> rgb,
      [BridgeEffectInstanceInfo? on]) {
    final comp = ui.selectedComp;
    final (lo, hi) = _colourRange(id);
    final was = _value(id, on);
    BridgeScalar at(BridgeScalar? s, double v) {
      final clamped = v.clamp(lo, hi).toDouble();
      return s == null || comp == null
          ? BridgeScalar.static_(clamped)
          : scalarWithValueAt(s, clamped, comp, ui.playheadFrame.value);
    }

    final c = was is BridgeEffectValue_Colour ? was.field0 : null;
    return BridgeEffectValue.colour(BridgeColour(
      r: at(c?.r, rgb[0]),
      g: at(c?.g, rgb[1]),
      b: at(c?.b, rgb[2]),
      a: c?.a ?? const BridgeScalar.static_(1),
    ));
  }

  /// Rows [ids] of the current node back to their declared defaults on every
  /// target, as one step. A keyed value takes the default at the playhead,
  /// as an Effect controls row's reset does, so the animation stays.
  void _reset(Iterable<String> ids) {
    if (_node == null) return;
    final rows = ids.where(_changed).toList();
    if (rows.isEmpty) return;
    _touchStart();
    _writeMany({
      for (final id in rows)
        if (_kind(id) case final kind?)
          if (defaultEffectValue(kind) case final d?)
            id: (on) => _defaultAtPlayhead(_value(id, on), d),
    });
  }

  /// [d], the declared default, written into [was] at the playhead when
  /// [was] is keyed; [d] itself otherwise.
  BridgeEffectValue _defaultAtPlayhead(
      BridgeEffectValue? was, BridgeEffectValue d) {
    final comp = ui.selectedComp;
    if (comp == null) return d;
    final frame = ui.playheadFrame.value;
    double of(BridgeScalar s) => sampledScalar(s, timeOfFrame(comp, frame));
    return switch ((was, d)) {
      (
        BridgeEffectValue_Float(field0: final BridgeScalar_Keyframed k),
        BridgeEffectValue_Float(:final field0)
      ) =>
        BridgeEffectValue.float(scalarWithValueAt(k, of(field0), comp, frame)),
      (
        BridgeEffectValue_Colour(field0: final c),
        BridgeEffectValue_Colour(field0: final dc)
      )
          when [c.r, c.g, c.b, c.a].any((s) => s is BridgeScalar_Keyframed) =>
        BridgeEffectValue.colour(BridgeColour(
          r: scalarWithValueAt(c.r, of(dc.r), comp, frame),
          g: scalarWithValueAt(c.g, of(dc.g), comp, frame),
          b: scalarWithValueAt(c.b, of(dc.b), comp, frame),
          a: scalarWithValueAt(c.a, of(dc.a), comp, frame),
        )),
      _ => d,
    };
  }

  /// Every row of the node bar what it is, back to its default.
  void _resetNode() =>
      _reset(_schema.keys.where((id) => !_structure.contains(id)));

  /// Every grade node on every target back to its defaults, as one step.
  void _resetGrade() {
    final targets = _targets;
    if (targets.isEmpty) return;
    asOneUndoStep(app.project, () {
      for (final s in targets) {
        final handles = _handles(s);
        for (final info in gradeNodesOf(_stackOf(s))) {
          final schema = _schemaOf(info.name);
          final values = <String, BridgeEffectValue>{
            for (final p in schema.values)
              if (!_structure.contains(p.id))
                if (defaultEffectValue(p.kind) case final d?)
                  p.id: _defaultAtPlayhead(_value(p.id, info), d),
          };
          for (final h in handles) {
            if (h.id() != info.id) continue;
            for (final v in values.entries) {
              h.setValue(id: v.key, value: v.value);
            }
          }
        }
        try {
          _commitHandles(s, handles);
        } catch (_) {
          // A locked layer refuses, and the rest still take it.
        }
      }
    }, name: 'Reset grade');
    _tell();
  }

  /// [run] on every target with the grade nodes [pick] chooses there, as one
  /// step.
  void _eachTarget(String name, Set<UuidValue> Function(GradeShot s) pick,
      void Function(GradeShot s, BridgeEffectInstance h) run) {
    final targets = _targets;
    if (targets.isEmpty) return;
    asOneUndoStep(app.project, () {
      for (final s in targets) {
        final ids = pick(s);
        for (final h in _handles(s)) {
          if (ids.contains(h.id())) run(s, h);
        }
      }
    }, name: name);
    _tell();
  }

  /// The current node on [s], by [_ensureOn]'s rule, without adding one. A
  /// shared node is its reference in the shot.
  Set<UuidValue> _nodeOn(GradeShot s) {
    final node = _node;
    if (node == null) return const {};
    final shown = _shot;
    if (shown != null && s.same(shown)) return {_sharedRef?.id ?? node.id};
    final mine = _stack.where((e) => e.name == node.name).toList().indexOf(node);
    final theirs = _stackOf(s).where((e) => e.name == node.name).toList();
    return mine >= 0 && mine < theirs.length ? {theirs[mine].id} : const {};
  }

  void _bypassNode(bool off) => _eachTarget('Bypass node', _nodeOn,
      (s, h) => s.layer.setEffectEnabled(effect: h, enabled: !off));

  void _bypassGrade(bool off) => _eachTarget('Bypass grade', _gradeIds,
      (s, h) => s.layer.setEffectEnabled(effect: h, enabled: !off));

  void _removeNode() {
    _eachTarget('Delete node', _nodeOn, (s, h) => s.layer.removeEffect(effect: h));
    ui.grade.setNode(null);
  }

  void _removeGrade() {
    _eachTarget(
        'Remove grade', _gradeIds, (s, h) => s.layer.removeEffect(effect: h));
    ui.grade.setNode(null);
  }

  /// A new node after the current one, made current. On the shot shown only:
  /// node structure is per shot.
  void _addNode(GradeNodeKind kind) {
    final shot = _shot;
    if (shot == null || !_canAdd(kind)) return;
    final scope = _scopeOn(shot);
    UuidValue? made;
    asOneUndoStep(app.project, () {
      made = scope != null
          ? _addToGroup(shot, scope, correctionName)
          : _engine.addNode(
              shot, _node?.id ?? gradeNodesOf(_stack).lastOrNull?.id, kind);
    }, name: 'Add node');
    if (made != null) ui.grade.setNode(made);
    _tell();
  }

  /// Whether a [kind] node can be added now: a group's grade takes serial
  /// nodes only.
  bool _canAdd(GradeNodeKind kind) {
    final shot = _shot;
    if (shot == null) return false;
    return _scopeOn(shot) != null
        ? kind == GradeNodeKind.serial
        : _engine.canAdd(kind, shot);
  }

  /// Pick the current node in Effect controls and bring the panel up.
  void _reveal({bool whole = false}) {
    final shot = _shot;
    if (shot == null) return;
    final ids = whole ? _gradeIds().toList() : [if (_node case final n?) n.id];
    if (ids.isEmpty) return;
    if (shot.clip == null) ui.setEffectSelection(shot.layer, ids);
    setPanelVisible(ui.split, Panel.effectControls, true);
    activatePanelTab(ui.split, Panel.effectControls);
    ui.activePane.value = Panel.effectControls.pane();
    ui.workspace.touch();
  }

  // --- Before ------------------------------------------------------------------

  /// Whether the before picture is being held up.
  bool _before = false;

  /// The picture without the grade, for as long as the button is held: the
  /// same preview path a drag uses, with the grade's nodes switched off in
  /// the copy, so the document and its history never see it.
  void _beforeDown() {
    final shot = _shot;
    final comp = ui.selectedComp;
    if (shot == null || comp == null) return;
    // The shot's own grade and its group's, whichever grade is in view.
    final ids = shotGradeIds(ui, shot).toSet();
    final handles = shotHandles(shot);
    for (final h in handles) {
      if (ids.contains(h.id()) || h.name() == 'grade_group') {
        h.setEnabled(enabled: false);
      }
    }
    comp.renderFrameWithPreview(
      frame: ui.playheadFrame.value,
      scale: ui.viewerScale,
      layer: shot.layer,
      effects: handles,
    );
    setState(() => _before = true);
  }

  void _beforeUp() {
    if (!_before) return;
    setState(() => _before = false);
    ui.requestFrame();
  }

  // --- Menus -------------------------------------------------------------------

  MenuRow _row(String key, String label, VoidCallback close, VoidCallback run,
          {bool ticked = false, bool enabled = true, String? action}) =>
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

  /// A row's or a page's menu: what it can do now, with a row's key and
  /// expression rows between, then the node's and the whole grade's rows. A
  /// row that could do nothing is left out.
  List<Widget> _menuFor(VoidCallback close,
      {Iterable<String> rows = const [], String? param}) {
    final menu = param == null ? null : _paramMenu(param);
    final changed = _anyChanged(rows);
    final aside = _aside;
    return menuGroups([
      [
        if (changed)
          _row('grade-row-reset', l10n.reset, close,
              () => _within(aside, () => _reset(rows))),
      ],
      if (menu != null) paramCopyRows(context, menu, close, tickColumn: true),
      if (menu != null) paramKeyRows(context, menu, close, tickColumn: true),
      if (menu != null) paramLastRows(context, menu, close, tickColumn: true),
      if (menu != null)
        paramControlRows(context, menu, close, tickColumn: true),
      // The node's and the grade's rows are the current node's, whatever
      // the row was aimed at.
      ..._within(null, () => _groundGroups(close)),
    ]);
  }

  /// The panel's own menu: the node's rows, then the grade's.
  List<Widget> _groundMenu(VoidCallback close) =>
      menuGroups(_groundGroups(close));

  List<List<Widget>> _groundGroups(VoidCallback close) {
    final node = _node;
    final shot = _shot;
    if (shot == null || !_gradeable(shot)) return const [];
    final off = _gradeOff;
    final placed = _sharedRef ?? node;
    return [
      [
        if (_nodeChanged)
          _row('grade-node-reset', l10n.gradeResetNode, close, _resetNode),
        if (placed != null)
          _row('grade-node-bypass', l10n.gradeBypassNode, close,
              () => _bypassNode(placed.enabled),
              ticked: !placed.enabled),
        if (node != null && _scopeOn(shot) == null && _shared == null)
          _row('grade-node-reveal', l10n.gradeRevealInEffectControls, close,
              _reveal),
        if (node != null)
          _row('grade-node-remove', l10n.gradeDeleteNode, close, _removeNode),
      ],
      [
        for (final kind in GradeNodeKind.values)
          _row('grade-node-add-${kind.name}', _kindLabel(kind), close,
              () => _addNode(kind),
              enabled: _canAdd(kind)),
      ],
      [
        if (_anyPresent)
          _row('grade-menu-reset', l10n.gradeResetAll, close, _resetGrade),
        if (_anyPresent)
          _row('grade-menu-bypass', l10n.gradeBypassAll, close,
              () => _bypassGrade(!off),
              ticked: off),
      ],
      [
        _row('grade-menu-copy', l10n.gradeCopy, close, () => copyGrade(ui),
            enabled: canCopyGrade(ui), action: 'grade.copy'),
        _row('grade-menu-paste', l10n.gradePaste, close,
            () => pasteGrade(app, ui),
            enabled: canPasteGrade(ui), action: 'grade.paste'),
      ],
      [
        if (_anyPresent)
          _row('grade-menu-remove', l10n.gradeRemoveAll, close, _removeGrade),
      ],
    ];
  }

  static String _kindLabel(GradeNodeKind kind) => switch (kind) {
        GradeNodeKind.serial => l10n.gradeAddSerial,
        GradeNodeKind.parallel => l10n.gradeAddParallel,
        GradeNodeKind.layer => l10n.gradeAddLayer,
        GradeNodeKind.outside => l10n.gradeAddOutside,
      };

  /// The key and expression rows of one parameter's menu: the same rows its
  /// Effect controls row has, acting on the same keys and the same curve. A
  /// shot with no node yet reads the default, and a write adds the node as a
  /// drag does.
  ParamMenu? _paramMenu(String id) {
    final comp = ui.selectedComp;
    final shot = _shot;
    final at = _node;
    final kind = _kind(id);
    final v = _value(id) ?? (kind == null ? null : defaultEffectValue(kind));
    if (comp == null || shot == null || v == null) return null;
    final scalars = switch (v) {
      BridgeEffectValue_Float(:final field0) => [field0],
      BridgeEffectValue_Colour(:final field0) => [
          field0.r,
          field0.g,
          field0.b,
          field0.a
        ],
      _ => null,
    };
    if (scalars == null) return null;
    final expression = switch (v) {
      BridgeEffectValue_Float(field0: final BridgeScalar_Expression e) => e,
      _ => null,
    };
    BridgeEffectValue rebuilt(List<BridgeScalar> next) =>
        v is BridgeEffectValue_Colour
            ? BridgeEffectValue.colour(
                BridgeColour(r: next[0], g: next[1], b: next[2], a: next[3]))
            : BridgeEffectValue.float(next[0]);
    final colour = v is BridgeEffectValue_Colour;
    final aside = _aside;
    // Each row is one Grade step, the node it may add included.
    void write(BridgeEffectValue Function(BridgeEffectInstanceInfo? on) valueFor,
        {bool all = true}) {
      _within(aside, () {
        _touchStart();
        _write(id, valueFor, all: all);
      });
    }

    return ParamMenu(
      // A row of the shot's own node, once there is one: a layer's rows can be
      // a template's controls, a clip's can't.
      control: switch ((at, _schema[id])) {
        (final at?, final param?) when shot.clip == null && _shared == null =>
          switch (
              paramControlRow(shot.layer.internallayerId, at.id, param)) {
            final row? => ControlRow(comp, row),
            null => null,
          },
        _ => null,
      },
      // The value as text, as Effect controls copies it, so it pastes into any
      // other row or field, and onto every picked shot.
      copy: expression != null
          ? null
          : () => _within(aside,
              () => copiedAxes(colour ? _colour(id).take(3) : [_float(id)])),
      paste: expression != null
          ? null
          : (text) {
              final n = pastedAxes(text, colour ? 3 : 1);
              if (n == null) return;
              write((on) =>
                  colour ? _colourFor(id, n, on) : _floatFor(id, n[0], on));
            },
      keys: expression != null
          ? null
          : KeyframeControlsFrb(
              scalars: scalars,
              comp: comp,
              onSeek: (frame) => ui.playheadFrame.value = frame,
              rowKey: 'grade-$_nodeName-$id',
              onWrite: (next) => write((_) => rebuilt(next), all: false),
            ),
      setExpression: v is BridgeEffectValue_Float && expression == null
          ? () => write(
              (_) => BridgeEffectValue.float(BridgeScalar.expression(
                  _float(id).toString(), newExpressionLanguage(context))),
              all: false)
          : null,
      removeExpression: expression == null
          ? null
          : () => write(
              (_) => BridgeEffectValue.float(BridgeScalar.static_(
                  sampleScalarWithContext(
                      scalar: expression,
                      time: timeOfFrame(comp, ui.playheadFrame.value),
                      layer: shot.layer))),
              all: false),
      // The Graph editor follows a layer's own effects, not a clip's.
      graphPath: at == null ||
              shot.clip != null ||
              _scopeOn(shot) != null ||
              _shared != null
          ? null
          : '${effectPath('${shot.layer.internallayerId}', '${at.id}')}/$id',
    );
  }
}
