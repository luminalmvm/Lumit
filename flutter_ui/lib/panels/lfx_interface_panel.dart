// An LFX effect's own interface: the extension's page in a panel that floats
// over the work, one per effect type. It follows the selected layer's
// instance of that effect, the picked one when there are several, and its
// page drives that instance's real rows through the `effect` calls.

import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/extensions.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/dialog_frame.dart';
import '../state/comp_time.dart';
import '../state/extension_host.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart';
import 'extension_panel.dart';
import 'keyframe_controls_frb.dart' show scalarWithValueAt;

/// The interfaces open now, by match name, so a second press does not open
/// a second one.
final Set<String> _open = {};

/// Open the interface of the LFX effect [matchName], titled [title], unless
/// it is open already. Reads the extension's folder, so it is asked on the
/// press and never in a build.
Future<void> showLfxInterface(BuildContext context,
    {required String matchName, required String title}) async {
  if (_open.contains(matchName)) return;
  final ui = context.read<LumitUiState>();
  final owner = ui.extensions.carrying(matchName)?.$1;
  BridgeLfxInterface? page;
  try {
    page = lfxInterface(matchName: matchName);
  } catch (_) {
    // No engine to ask, as in a widget test.
  }
  if (owner == null || page == null) return;
  // The extension as the page host knows it, showing this page.
  final extension = BridgeExtension(
    id: owner.id,
    name: owner.name,
    version: owner.version,
    summary: owner.summary,
    author: owner.author,
    homepage: owner.homepage,
    folder: page.folder,
    panel: true,
    entry: page.entry,
    permissions: owner.permissions,
    hosts: owner.hosts,
    broken: false,
    effects: owner.effects,
  );
  _open.add(matchName);
  try {
    await showLumitModal<void>(
      context: context,
      id: 'lfx-interface-$matchName',
      modeless: true,
      dimBackground: false,
      initialSize: Size(page.width.toDouble(),
          page.height.toDouble() + dialogTitleStrip + 1),
      minSize: const Size(200, 120),
      builder: (close) => LfxInterfaceWindow(
        extension: extension,
        matchName: matchName,
        title: title,
        onClose: () => close(null),
      ),
    );
  } finally {
    _open.remove(matchName);
  }
}

/// The floating panel itself: its title strip, and the page while there is
/// an instance to follow.
class LfxInterfaceWindow extends StatefulWidget {
  final BridgeExtension extension;
  final String matchName;
  final String title;
  final VoidCallback onClose;

  const LfxInterfaceWindow({
    super.key,
    required this.extension,
    required this.matchName,
    required this.title,
    required this.onClose,
  });

  @override
  State<LfxInterfaceWindow> createState() => _LfxInterfaceWindowState();
}

class _LfxInterfaceWindowState extends State<LfxInterfaceWindow> {
  late final LfxFollow _follow = LfxFollow(
      ui: context.read<LumitUiState>(), matchName: widget.matchName);

  @override
  void dispose() {
    _follow.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      children: [
        dialogTitleBar(t,
            title: widget.title,
            onClose: widget.onClose,
            keyPrefix: 'lfx-interface'),
        Expanded(
          child: ListenableBuilder(
            listenable: _follow,
            builder: (context, _) => _follow.target == null
                ? Center(
                    child: Text(
                      l10n.lfxInterfaceEmpty(widget.title),
                      key: const ValueKey('lfx-interface-empty'),
                      style: t.small.copyWith(color: t.textMuted),
                    ),
                  )
                // A drag in the page is the page's own, so turning a knob in
                // it never moves the window.
                : GestureDetector(
                    onPanStart: (_) {},
                    child: ExtensionView(
                      key: ValueKey<String>('lfx-page-${widget.matchName}'),
                      extension: widget.extension,
                      effect: _follow,
                    ),
                  ),
          ),
        ),
      ],
    );
  }
}

/// The instance an interface follows, and the `effect` calls its page makes.
///
/// Rows, the preview while dragging and the commit go through the same
/// [EffectStackEditor] calls the Effect controls rows use, so a drag is one
/// undo step and the engine clamps every value.
class LfxFollow extends ChangeNotifier implements EffectChannel {
  final LumitUiState ui;
  final String matchName;

  LfxFollow({required this.ui, required this.matchName}) {
    ui.selectedLayer.addListener(_look);
    ui.selectedEffects.addListener(_look);
    ui.playheadFrame.addListener(_look);
    ui.model.addListener(_look);
    _look();
  }

  @override
  void Function(String event, Map<String, Object?> data)? onEvent;

  /// The layer and the instance on it being followed, or null.
  ({LayerReference layer, BridgeEffectInstanceInfo info})? target;

  final EffectStackEditor _editor = EffectStackEditor();

  /// A drag is running, and what it has set so far.
  bool _dragging = false;
  final Map<String, BridgeEffectValue> _staged = {};

  /// The values last told to the page, so only a change is told.
  String? _told;

  @override
  void dispose() {
    ui.selectedLayer.removeListener(_look);
    ui.selectedEffects.removeListener(_look);
    ui.playheadFrame.removeListener(_look);
    ui.model.removeListener(_look);
    _editor.clear();
    super.dispose();
  }

  ({LayerReference layer, BridgeEffectInstanceInfo info})? _find() {
    final layer = ui.selectedLayer.value;
    if (layer == null) return null;
    final effects = ui.model.byId(layer.internallayerId)?.info.effects ?? [];
    final mine = [
      for (final e in effects)
        if (e.name == matchName) e,
    ];
    if (mine.isEmpty) return null;
    final picked = ui.selectedEffects.value;
    final info = mine.firstWhere((e) => picked.contains(e.id),
        orElse: () => mine.first);
    return (layer: layer, info: info);
  }

  void _look() {
    final next = _find();
    final was = target;
    target = next;
    if (next?.info.id != was?.info.id ||
        next?.layer.internallayerId != was?.layer.internallayerId) {
      _editor.clear();
      _dragging = false;
      _staged.clear();
      _told = null;
      notifyListeners();
      onEvent?.call('effect.target', {'id': next?.info.id.toString()});
    }
    if (next == null) return;
    final values = _values(next.info);
    final key = jsonEncode(values);
    if (key == _told) return;
    final first = _told == null;
    _told = key;
    if (!first) onEvent?.call('effect.changed', {'values': values});
  }

  /// The rows a page can drive: numbers, checkboxes, choices, seeds and
  /// colours. A layer, a file or a text row is left out.
  List<BridgeParamInfo> _rows(BridgeEffectInstanceInfo info) => [
        for (final row in relistedRows(
            [...cachedListParameters(matchName), ...info.derivedParams],
            info.rowOptions))
          if (!info.hiddenRows.contains(row.id) && _kind(row.kind) != null) row,
      ];

  static String? _kind(BridgeParamKind kind) => switch (kind) {
        BridgeParamKind_Float() => 'float',
        BridgeParamKind_Int() => 'int',
        BridgeParamKind_Angle() => 'angle',
        BridgeParamKind_Slider() => 'slider',
        BridgeParamKind_Bool() => 'bool',
        BridgeParamKind_Choice() => 'choice',
        BridgeParamKind_Seed() => 'seed',
        BridgeParamKind_Colour() => 'colour',
        _ => null,
      };

  /// A scalar at the playhead. Null for one an expression drives.
  double? _at(BridgeScalar scalar) {
    final comp = ui.selectedComp;
    if (comp == null) return null;
    return switch (scalar) {
      BridgeScalar_Static(:final field0) => field0,
      BridgeScalar_Keyframed() =>
        sampledScalar(scalar, timeOfFrame(comp, ui.playheadFrame.value)),
      _ => null,
    };
  }

  Object? _out(BridgeEffectValue? value) => switch (value) {
        BridgeEffectValue_Float(:final field0) => _at(field0),
        BridgeEffectValue_Bool(:final field0) => field0,
        BridgeEffectValue_Choice(:final field0) => field0,
        BridgeEffectValue_Seed(:final field0) => field0,
        BridgeEffectValue_Colour(:final field0) => [
            _at(field0.r),
            _at(field0.g),
            _at(field0.b),
            _at(field0.a),
          ],
        _ => null,
      };

  BridgeEffectValue? _stored(BridgeEffectInstanceInfo info, String id) {
    for (final v in info.values) {
      if (v.id == id) return v.value;
    }
    return null;
  }

  Map<String, Object?> _values(BridgeEffectInstanceInfo info) => {
        for (final row in _rows(info)) row.id: _out(_stored(info, row.id)),
      };

  ({LayerReference layer, BridgeEffectInstanceInfo info}) _target() =>
      target ??
      (throw const ExtensionRefusal('failed', 'no instance is selected'));

  BridgeParamInfo _row(BridgeEffectInstanceInfo info, String id) {
    for (final row in _rows(info)) {
      if (row.id == id) return row;
    }
    throw ExtensionRefusal('bad-params', 'there is no row $id');
  }

  @override
  List<Map<String, Object?>> rows() {
    final info = _target().info;
    return [
      for (final row in _rows(info))
        {
          'id': row.id,
          'label': row.label,
          'kind': _kind(row.kind),
          ...switch (row.kind) {
            BridgeParamKind_Float(:final sliderMin, :final sliderMax) => {
                'min': sliderMin,
                'max': sliderMax,
              },
            BridgeParamKind_Int(:final sliderMin, :final sliderMax) => {
                'min': sliderMin.toInt(),
                'max': sliderMax.toInt(),
              },
            BridgeParamKind_Slider(:final min, :final max) ||
            BridgeParamKind_Colour(:final min, :final max) =>
              {'min': min, 'max': max},
            _ => {'min': null, 'max': null},
          },
          'choices': switch (row.kind) {
            BridgeParamKind_Choice(:final options) => options,
            _ => null,
          },
          'value': _out(_stored(info, row.id)),
        },
    ];
  }

  @override
  void begin(String id) {
    _row(_target().info, id);
    _dragging = true;
  }

  @override
  void set(String id, Object? value) {
    final (:layer, :info) = _target();
    final row = _row(info, id);
    final written = _value(row, _stored(info, id), value);
    final comp = ui.selectedComp;
    if (comp == null) return;
    if (_dragging) {
      _staged[id] = written;
      _editor.live(comp, layer, info.id, id, written,
          frame: ui.playheadFrame.value, scale: ui.viewerScale);
    } else {
      _editor.write(layer, info.id, id, written);
      ui.model.refresh();
    }
  }

  @override
  void end() {
    _dragging = false;
    final held = target;
    if (held == null || _staged.isEmpty) return;
    _editor.writeAll(held.layer, held.info.id, Map.of(_staged));
    _staged.clear();
    ui.model.refresh();
  }

  /// [value] from the page as the row's own value, keyed at the playhead
  /// where the row is keyed.
  BridgeEffectValue _value(
      BridgeParamInfo row, BridgeEffectValue? stored, Object? value) {
    final comp = ui.selectedComp!;
    final frame = ui.playheadFrame.value;
    BridgeScalar scalar(BridgeScalar? was, Object? v) {
      if (v is! num) {
        throw ExtensionRefusal('bad-params', '${row.id} takes a number');
      }
      if (was is BridgeScalar_Expression) {
        throw ExtensionRefusal(
            'failed', '${row.id} is driven by an expression');
      }
      final n = row.kind is BridgeParamKind_Int ? v.roundToDouble() : v;
      return scalarWithValueAt(
          was ?? BridgeScalar.static_(0), n.toDouble(), comp, frame);
    }

    switch (row.kind) {
      case BridgeParamKind_Bool():
        if (value is bool) return BridgeEffectValue.bool(value);
        throw ExtensionRefusal('bad-params', '${row.id} takes true or false');
      case BridgeParamKind_Choice(:final options):
        if (value is int && value >= 0 && value < options.length) {
          return BridgeEffectValue.choice(value);
        }
        throw ExtensionRefusal('bad-params', '${row.id} takes an option index');
      case BridgeParamKind_Seed():
        if (value is int) return BridgeEffectValue.seed(value);
        throw ExtensionRefusal('bad-params', '${row.id} takes a whole number');
      case BridgeParamKind_Colour():
        final was = stored is BridgeEffectValue_Colour ? stored.field0 : null;
        if (value is! List || value.length < 3 || value.length > 4) {
          throw ExtensionRefusal('bad-params', '${row.id} takes [r, g, b, a]');
        }
        return BridgeEffectValue.colour(BridgeColour(
          r: scalar(was?.r, value[0]),
          g: scalar(was?.g, value[1]),
          b: scalar(was?.b, value[2]),
          a: scalar(was?.a, value.length > 3 ? value[3] : 1),
        ));
      default:
        final was = stored is BridgeEffectValue_Float ? stored.field0 : null;
        return BridgeEffectValue.float(scalar(was, value));
    }
  }
}
