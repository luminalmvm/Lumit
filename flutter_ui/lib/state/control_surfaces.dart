// Control surfaces: Tangent panels and MIDI controllers driving the grade.
//
// The engine talks to the hardware and hands every turn and press over on
// one stream. This applies them to the current grade node through a
// [GradeSurfaceTarget], keeps the transport and undo keys for itself, and
// pushes back what the panels' displays should show. The choices (Tangent on,
// which MIDI ports, what each learned control drives) live in the workspace
// settings like any other preference.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:lumit_flutter/src/rust/api/surface.dart';

import '../l10n/strings.dart';
import '../panels/correction_surface_target_frb.dart';
import 'app_state.dart';
import 'ui_state.dart';

/// The grade a surface drives: the Grade panel's current node on the current
/// shot. [CorrectionSurfaceTarget] is the one there is.
abstract class GradeSurfaceTarget {
  /// Move Correction row [param] by [by], in the row's own units, without
  /// committing it: a burst of these is one live drag. [axis] is 0 for a
  /// wheel's chroma across, 1 for up, 2 for its master, and null for a row
  /// that is one number.
  void nudge(String param, int? axis, double by);

  /// Put [param] back where it rests, as one undo step: only [axis] of a
  /// wheel when given.
  void reset(String param, [int? axis]);

  /// Run the action [id]: an id from `surface_targets`, or `palette_` and
  /// the Grade panel's page name (`palette_curves`), which a panel's page
  /// menu steps through.
  void action(String id);

  /// What the panels show: each row's value by key (`contrast`, `lift.x`,
  /// `lift.y`, `lift.master`), the place from 0 of the Grade panel's page
  /// under `grade_page` and of its wheel mode under `wheel_mode`, and the
  /// node's number from 1 under `node`. Staged values count.
  Map<String, double> values();

  /// Write what [nudge] staged as one undo step.
  void commit();

  /// The current shot's name, for the panels' status line.
  String shot();
}

/// How long the hardware has to be still before a drag is written.
const Duration surfaceQuiet = Duration(milliseconds: 250);

/// How often at most the panels' displays are brought up to date.
const Duration _showEvery = Duration(milliseconds: 50);

/// The Grade panel's pages, in the order its page menu steps through them.
const List<String> _palettes = [
  'wheels',
  'adjust',
  'curves',
  'qualifier',
  'windows',
  'match',
  'look',
];

/// The wheel modes, in the order the wheel mode menu steps through them.
const List<String> _wheelModes = ['wheels', 'bars', 'log', 'hdr'];

/// The surfaces, for the whole run. Made by [LumitUiState].
class ControlSurfaces {
  ControlSurfaces(this._ui, this._app);

  final LumitUiState _ui;
  final LumitState _app;

  /// The grade the surfaces drive.
  late final GradeSurfaceTarget _target = CorrectionSurfaceTarget(_ui, _app);

  /// Where Tangent panels stand.
  final ValueNotifier<BridgeTangentStatus> tangent =
      ValueNotifier(const BridgeTangentStatus.off());

  /// The MIDI input ports on the system now.
  final ValueNotifier<List<String>> ports = ValueNotifier(const []);

  /// What MIDI learn will tie the next control moved to, or null when not
  /// learning.
  final ValueNotifier<BridgeSurfaceTarget?> learning = ValueNotifier(null);

  /// The learned controls, as the workspace keeps them.
  List<BridgeMidiMapping> get mappings => [
        for (final json in _ui.workspace.midiMappings)
          if (_mappingFrom(json) case final mapping?) mapping,
      ];

  Timer? _quiet;
  Timer? _show;
  bool _staged = false;

  /// Listen for the whole run, and turn on what the workspace says is on.
  void start() {
    final sink = RustStreamSink<BridgeSurfaceEvent>();
    // The sink has no stream until the call has set it up.
    unawaited(listenToSurfaces(events: sink));
    sink.stream.listen(_on);
    _app.onChange.listen((_) => _showSoon());
    _ui.grade.shot.addListener(_showSoon);
    _ui.grade.node.addListener(_showSoon);
    _ui.grade.page.addListener(_showSoon);
    _ui.grade.wheelMode.addListener(_showSoon);
    _ui.playheadFrame.addListener(_showSoon);
    final workspace = _ui.workspace;
    if (workspace.tangentPanels) setTangentPanels(enabled: true);
    if (workspace.midiInputs.isNotEmpty) {
      openMidiInputs(names: workspace.midiInputs);
      setMidiMappings(mappings: mappings);
    }
  }

  // --- Settings --------------------------------------------------------------

  void setTangent(bool on) {
    _ui.workspace.setTangentPanels(on);
    setTangentPanels(enabled: on);
    if (!on) tangent.value = const BridgeTangentStatus.off();
  }

  /// The ports on the system, from now on kept up to date in [ports].
  void watchPorts() => ports.value = midiInputs();

  void setInput(String port, bool on) {
    final names = [
      for (final name in _ui.workspace.midiInputs)
        if (name != port) name,
      if (on) port,
    ];
    _ui.workspace.setMidiInputs(names);
    openMidiInputs(names: names);
    setMidiMappings(mappings: mappings);
  }

  /// Tie the next control moved on an open port to [to].
  void learn(BridgeSurfaceTarget to) {
    learning.value = to;
    midiLearn(listening: true);
  }

  void stopLearning() {
    learning.value = null;
    midiLearn(listening: false);
  }

  void remove(BridgeMidiMapping mapping) =>
      _keep([for (final m in mappings) if (m != mapping) m]);

  void setEncoding(BridgeMidiMapping mapping, BridgeMidiEncoding encoding) =>
      _keep([
        for (final m in mappings)
          m == mapping
              ? BridgeMidiMapping(
                  control: m.control, encoding: encoding, target: m.target)
              : m,
      ]);

  void _keep(List<BridgeMidiMapping> kept) {
    _ui.workspace.setMidiMappings([for (final m in kept) _mappingJson(m)]);
    setMidiMappings(mappings: kept);
  }

  // --- Events ----------------------------------------------------------------

  void _on(BridgeSurfaceEvent event) {
    switch (event) {
      case BridgeSurfaceEvent_Nudge(:final param, :final axis, :final by):
        _target.nudge(param, axis?.index, by);
        _staged = true;
        _quiet?.cancel();
        _quiet = Timer(surfaceQuiet, _commit);
        _showSoon();
      case BridgeSurfaceEvent_Reset(:final param, :final axis):
        _commit();
        _target.reset(param, axis?.index);
        _showSoon();
      case BridgeSurfaceEvent_Action(:final id, down: true):
        _commit();
        _act(id);
        _showSoon();
      case BridgeSurfaceEvent_Action():
        break;
      case BridgeSurfaceEvent_Menu(:final id, :final step):
        _commit();
        _menu(id, step);
        _showSoon();
      case BridgeSurfaceEvent_Jog(:final frames):
        _ui.stepFrame(frames);
      case BridgeSurfaceEvent_Shuttle(:final step):
        _ui.shuttle(step);
      case BridgeSurfaceEvent_Tangent(:final status):
        tangent.value = status;
        if (status is BridgeTangentStatus_Connected) _showSoon();
      case BridgeSurfaceEvent_Ports(:final ports):
        this.ports.value = ports;
      case BridgeSurfaceEvent_Learned(:final control, :final encoding):
        final to = learning.value;
        learning.value = null;
        if (to == null) return;
        // One control drives one thing: learning it again replaces it.
        _keep([
          for (final m in mappings)
            if (m.control != control) m,
          BridgeMidiMapping(control: control, encoding: encoding, target: to),
        ]);
    }
  }

  /// The transport and undo are the window's, whatever grade is current.
  void _act(String id) {
    switch (id) {
      case 'play_stop':
        _ui.requestTogglePlay();
      case 'frame_back':
        _ui.stepFrame(-1);
      case 'frame_forward':
        _ui.stepFrame(1);
      case 'undo' || 'redo':
        _app.settleHistory();
        id == 'undo' ? _app.project?.undo() : _app.project?.redo();
        _app.notifyDocumentChanged();
      default:
        _target.action(id);
    }
  }

  /// Step a menu by [step], or back to its first item with 0.
  void _menu(String id, int step) {
    final (items, action) = switch (id) {
      'grade_page' => (_palettes, 'palette_'),
      'wheel_mode' => (_wheelModes, 'page_'),
      _ => (const <String>[], ''),
    };
    if (items.isEmpty) return;
    final at = (_target.values()[id] ?? 0).round();
    final next = step == 0 ? 0 : (at + step) % items.length;
    _target.action('$action${items[next]}');
  }

  /// Write the drag in hand, if there is one.
  void _commit() {
    _quiet?.cancel();
    _quiet = null;
    if (!_staged) return;
    _staged = false;
    _target.commit();
  }

  // --- The displays ----------------------------------------------------------

  void _showSoon() {
    if (tangent.value is! BridgeTangentStatus_Connected) return;
    _show ??= Timer(_showEvery, () {
      _show = null;
      showOnSurfaces(
        values: [
          for (final MapEntry(:key, :value) in _target.values().entries)
            BridgeSurfaceValue(key: key, value: value),
        ],
        shot: _target.shot(),
      );
    });
  }
}

/// A learned control as the settings file keeps it.
Map<String, dynamic> _mappingJson(BridgeMidiMapping m) => {
      'port': m.control.port,
      'channel': m.control.channel,
      'kind': m.control.kind.name,
      'number': m.control.number,
      'encoding': m.encoding.name,
      ...switch (m.target) {
        BridgeSurfaceTarget_Param(:final param, :final axis) => {
            'param': param,
            if (axis != null) 'axis': axis.name,
          },
        BridgeSurfaceTarget_Action(:final id) => {'action': id},
      },
    };

/// A learned control read back, or null for one this build can't read.
BridgeMidiMapping? _mappingFrom(Map<String, dynamic> j) {
  final kind = BridgeMidiKind.values.asNameMap()[j['kind']];
  final encoding = BridgeMidiEncoding.values.asNameMap()[j['encoding']];
  final (port, channel, number) = (j['port'], j['channel'], j['number']);
  if (kind == null ||
      encoding == null ||
      port is! String ||
      channel is! int ||
      number is! int) {
    return null;
  }
  final BridgeSurfaceTarget target;
  if (j['param'] case final String param) {
    target = BridgeSurfaceTarget.param(
        param: param, axis: BridgeSurfaceAxis.values.asNameMap()[j['axis']]);
  } else if (j['action'] case final String id) {
    target = BridgeSurfaceTarget.action(id: id);
  } else {
    return null;
  }
  return BridgeMidiMapping(
    control: BridgeMidiControl(
        port: port, channel: channel, kind: kind, number: number),
    encoding: encoding,
    target: target,
  );
}

/// What [target] is called in the MIDI learn list.
String surfaceTargetLabel(BridgeSurfaceTarget target) => switch (target) {
      BridgeSurfaceTarget_Param(:final param, :final axis) => switch (axis) {
          BridgeSurfaceAxis.x => l10n.surfaceWheelX(_rowLabel(param)),
          BridgeSurfaceAxis.y => l10n.surfaceWheelY(_rowLabel(param)),
          BridgeSurfaceAxis.master => l10n.surfaceWheelMaster(_rowLabel(param)),
          null => _rowLabel(param),
        },
      BridgeSurfaceTarget_Action(:final id) => _actionLabel(id),
    };

/// A Correction row's name. One this build has no name for shows its id.
String _rowLabel(String param) => switch (param) {
      'lift' => l10n.fxLift,
      'gamma' => l10n.fxGamma,
      'gain' => l10n.fxGain,
      'offset' => l10n.fxOffset,
      'shadow' => l10n.surfaceShadow,
      'midtone' => l10n.surfaceMidtone,
      'highlight' => l10n.surfaceHighlight,
      'low_range' => l10n.surfaceLowRange,
      'high_range' => l10n.surfaceHighRange,
      'contrast' => l10n.fxContrast,
      'pivot' => l10n.surfacePivot,
      'saturation' => l10n.fxSaturation,
      'hue' => l10n.surfaceHue,
      'lum_mix' => l10n.surfaceLumMix,
      'temperature' => l10n.fxTemperature,
      'tint' => l10n.fxTint,
      'colour_boost' => l10n.surfaceColourBoost,
      'shadows' => l10n.fxShadows,
      'highlights' => l10n.fxHighlights,
      'midtone_detail' => l10n.surfaceMidtoneDetail,
      'exposure' => l10n.fxExposure,
      _ => param,
    };

String _actionLabel(String id) => switch (id) {
      'shot_previous' => l10n.surfaceShotPrevious,
      'shot_next' => l10n.surfaceShotNext,
      'node_previous' => l10n.surfaceNodePrevious,
      'node_next' => l10n.surfaceNodeNext,
      'node_add_serial' => l10n.surfaceNodeAddSerial,
      'node_add_parallel' => l10n.surfaceNodeAddParallel,
      'node_add_layer' => l10n.surfaceNodeAddLayer,
      'node_add_outside' => l10n.surfaceNodeAddOutside,
      'node_bypass' => l10n.surfaceNodeBypass,
      'node_reset' => l10n.surfaceNodeReset,
      'grade_bypass' => l10n.surfaceGradeBypass,
      'still_grab' => l10n.surfaceStillGrab,
      'play_stop' => l10n.surfacePlayStop,
      'frame_back' => l10n.surfaceFrameBack,
      'frame_forward' => l10n.surfaceFrameForward,
      'undo' => l10n.menuUndo,
      'redo' => l10n.menuRedo,
      'page_wheels' => l10n.surfacePageWheels,
      'page_bars' => l10n.surfacePageBars,
      'page_log' => l10n.surfacePageLog,
      'page_hdr' => l10n.surfacePageHdr,
      final reset when reset.startsWith('reset_') =>
        l10n.surfaceResetWheel(_rowLabel(reset.substring('reset_'.length))),
      _ => id,
    };

/// A learned control, as in CC 21, channel 1.
String midiControlLabel(BridgeMidiControl c) => switch (c.kind) {
      BridgeMidiKind.note => l10n.settingsMidiNote(c.number, c.channel + 1),
      BridgeMidiKind.cc => l10n.settingsMidiCc(c.number, c.channel + 1),
      BridgeMidiKind.cc14 => l10n.settingsMidiCc14(c.number, c.channel + 1),
    };

String midiEncodingLabel(BridgeMidiEncoding e) => switch (e) {
      BridgeMidiEncoding.absolute => l10n.settingsMidiAbsolute,
      BridgeMidiEncoding.twosComplement => l10n.settingsMidiTwosComplement,
      BridgeMidiEncoding.signBit => l10n.settingsMidiSignBit,
      BridgeMidiEncoding.offset64 => l10n.settingsMidiOffset64,
    };
