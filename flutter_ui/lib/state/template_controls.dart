// The fronted composition's graphics template, held for the panels: which
// rows are controls, and the two edits that make and unmake one.
//
// Read when the composition's read model is: once for each document
// revision, and once more when a panel re-reads after an edit of its own,
// which is how a menu option being recorded is seen though it moves no
// revision. Kept between, so a build asks this and never the engine.

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/template.dart';
import 'package:uuid/uuid.dart';

import 'comp_model.dart';

/// A row a control can name, and the composition it is in.
class ControlRow {
  final CompositionReference comp;
  final BridgeRowRef row;

  /// The same row of the other layers a panel is editing with this one, which
  /// go with it.
  final List<BridgeRowRef> also;
  const ControlRow(this.comp, this.row, {this.also = const []});
}

/// Which row [row] is, as text. Two refs that name the same row give the same
/// key: everything but the label counts, and transform rows by their list.
/// Which rows a control sets is asked by this key.
String controlRowKey(BridgeRowRef row) => [
      row.kind.name,
      row.layer,
      row.props.map((p) => p.name).join(','),
      row.owner ?? '',
      row.param ?? '',
      row.paramY ?? '',
    ].join('|');

/// The fronted composition's template and its controls, by the row each sets.
/// Tells its listeners whenever it has read a template again.
class TemplateControls extends ChangeNotifier {
  CompModel? _model;
  CompositionReference? Function() _front = _none;
  VoidCallback? _edited;

  CompositionReference? _comp;
  BridgeTemplate? _template;
  Map<String, BridgeControl> _byRow = const {};

  static CompositionReference? _none() => null;

  /// Follow [model], the read model of the composition [front] answers.
  /// [edited] is told after an edit made here, so the rest of the app hears
  /// of it without waiting for the engine.
  void follow(CompModel model, CompositionReference? Function() front,
      {VoidCallback? edited}) {
    _model?.removeListener(_read);
    _model = model;
    _front = front;
    _edited = edited;
    model.addListener(_read);
    _read();
  }

  /// The composition this holds the template of.
  CompositionReference? get comp => _comp;

  /// The template, or null for a composition with no controls and for no
  /// composition at all.
  BridgeTemplate? get template => _template;

  /// The controls in the template's order.
  List<BridgeControl> get controls => _template?.controls ?? const [];

  /// Whether rows of [comp] can be made controls: it is the composition held
  /// here, and not a node graph, whose rows the engine cannot name.
  bool offers(CompositionReference comp) =>
      _comp == comp && !(_model?.heldIsNodeGraph ?? true);

  /// The control that sets [row], or null when the row is not one.
  BridgeControl? controlFor(BridgeRowRef row) =>
      _byRow.isEmpty ? null : _byRow[controlRowKey(row)];

  /// Whether any of the layer's own rows is a control: its visibility, its
  /// words or its footage.
  bool layerIsControl(UuidValue layer) =>
      _byRow.isNotEmpty &&
      controls.any((c) =>
          c.row.layer == layer &&
          (c.row.kind == BridgeRowKind.visible ||
              c.row.kind == BridgeRowKind.text ||
              c.row.kind == BridgeRowKind.source));

  /// Add to controls: every one of [rows] that is not a control already, as
  /// one undo step. False when nothing was added.
  bool add(List<BridgeRowRef> rows) {
    final comp = _comp;
    if (comp == null || rows.isEmpty) return false;
    final List<String> made;
    try {
      made = comp.addControls(rows: rows);
    } catch (_) {
      // The composition or a layer went away under the press.
      return false;
    }
    if (made.isEmpty) return false;
    _afterEdit();
    return true;
  }

  /// Remove from controls: the controls that set [rows], as one undo step.
  bool remove(List<BridgeRowRef> rows) {
    final comp = _comp;
    final ids = [
      for (final row in rows)
        if (controlFor(row) case final control?) control.id,
    ];
    if (comp == null || ids.isEmpty) return false;
    try {
      comp.removeControls(ids: ids);
    } catch (_) {
      return false;
    }
    _afterEdit();
    return true;
  }

  void _afterEdit() {
    _edited?.call();
    // Puts the edit on screen now. The read below rides on its notice.
    _model?.refresh();
  }

  void _read() {
    final comp = _front();
    _comp = comp;
    BridgeTemplate? next;
    try {
      // No revision is a composition that has gone.
      next = comp == null || _model?.heldRevision == null
          ? null
          : comp.getTemplate();
    } catch (_) {
      // The composition has gone. No template, as for none fronted.
      next = null;
    }
    if (next == null && _template == null) return;
    _template = next;
    _byRow = {
      for (final c in next?.controls ?? const <BridgeControl>[])
        controlRowKey(c.row): c,
    };
    notifyListeners();
  }

  @override
  void dispose() {
    _model?.removeListener(_read);
    super.dispose();
  }
}
