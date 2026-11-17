// The comp's loudness target and Normalise mix, shared by the Master strip's
// menu and Composition ▸ Audio.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart';

import '../l10n/strings.dart';
import '../shell/dialog_frame.dart';
import '../state/app_state.dart';
import '../widgets/controls.dart';

/// The common targets, in LUFS: streaming, podcasts and the two broadcast
/// standards.
const List<double> loudnessPresets = [-14, -16, -23, -24];

/// The true peak a preset target keeps under, in dBTP.
const double presetTruePeakDb = -1;

/// A target as its menu row reads.
String loudnessTargetLabel(double lufs) => l10n.audioLoudnessTarget(
    '−${(-lufs).toStringAsFixed(lufs.truncateToDouble() == lufs ? 0 : 1)}');

/// Set the comp's loudness target, or clear it with null, as one undo step.
void setCompLoudnessTarget(CompositionReference comp,
    BridgeLoudnessTarget? target, VoidCallback onChanged) {
  try {
    comp.setLoudnessTarget(target: target);
  } catch (_) {
    return;
  }
  onChanged();
}

/// A preset target.
BridgeLoudnessTarget presetTarget(double lufs) =>
    BridgeLoudnessTarget(lufs: lufs, truePeakDb: presetTruePeakDb);

/// Whether [target] is one of the presets, for the menu's tick.
bool isPreset(BridgeLoudnessTarget? target, double lufs) =>
    target != null &&
    target.lufs == lufs &&
    target.truePeakDb == presetTruePeakDb;

/// Ask for a loudness and a true peak, seeded with [current].
Future<BridgeLoudnessTarget?> askLoudnessTarget(
        BuildContext context, BridgeLoudnessTarget? current) =>
    showLumitModal<BridgeLoudnessTarget>(
      context: context,
      builder: (close) => _TargetBody(
        start: current ?? presetTarget(loudnessPresets.first),
        onConfirm: close,
        onCancel: () => close(null),
      ),
    );

/// Custom…: a target of the person's own.
Future<void> customLoudnessTarget(BuildContext context,
    CompositionReference comp, VoidCallback onChanged) async {
  BridgeLoudnessTarget? current;
  try {
    current = comp.loudnessTarget();
  } catch (_) {}
  final asked = await askLoudnessTarget(context, current);
  if (asked != null) setCompLoudnessTarget(comp, asked, onChanged);
}

/// Whether a Normalise mix is measuring now. The engine refuses a second
/// measure, and its refusal reaches Dart without saying why, so the busy case
/// is caught here before it is asked.
bool _normalising = false;

/// Normalise mix…: measure the whole mix and set the master so it meets the
/// comp's target, or one asked for when it has none, behind the busy card
/// with its Cancel. The status line says the loudness it lands at.
Future<void> normaliseMixCommand(BuildContext context, LumitState app,
    CompositionReference comp, VoidCallback onChanged) async {
  BridgeLoudnessTarget? target;
  try {
    target = comp.loudnessTarget();
  } catch (_) {
    return;
  }
  if (_normalising) {
    app.postNotice(l10n.mixerNormaliseBusy);
    return;
  }
  target ??= await askLoudnessTarget(context, null);
  if (target == null) return;
  final aim = target.lufs;
  // A run stopped by its Cancel says nothing; any other refusal is a mix
  // with nothing to measure.
  var stopped = false;
  _normalising = true;
  try {
    final reached = await app.withBusyCard(
      l10n.mixerNormalisingMix,
      (progress) => comp.normaliseMix(targetLufs: aim, progress: progress),
      cancel: () {
        stopped = true;
        cancelMeasureMix();
      },
    );
    onChanged();
    app.postNotice(l10n.mixerNormalisedMix(reached.toStringAsFixed(1)));
  } catch (_) {
    if (!stopped) app.postNotice(l10n.mixerNormaliseSilent);
  } finally {
    _normalising = false;
  }
}

class _TargetBody extends StatefulWidget {
  final BridgeLoudnessTarget start;
  final ValueChanged<BridgeLoudnessTarget> onConfirm;
  final VoidCallback onCancel;

  const _TargetBody(
      {required this.start, required this.onConfirm, required this.onCancel});

  @override
  State<_TargetBody> createState() => _TargetBodyState();
}

class _TargetBodyState extends State<_TargetBody> {
  late double _lufs = widget.start.lufs;
  late double _peak = widget.start.truePeakDb;

  void _confirm() =>
      widget.onConfirm(BridgeLoudnessTarget(lufs: _lufs, truePeakDb: _peak));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    Widget field(String key, String label, double value, double min, double max,
            String suffix, ValueChanged<double> set) =>
        dialogRow(
          t,
          label,
          SizedBox(
            height: dialogControlHeight,
            child: DragValueField(
              key: ValueKey<String>(key),
              value: value,
              min: min,
              max: max,
              decimals: 1,
              suffix: ' $suffix',
              onChanged: (v) => setState(() => set(v.toDouble())),
            ),
          ),
          labelColumn: 120,
        );
    return dialogEnter(
      onEnter: _confirm,
      child: DialogFrame(
        width: 320,
        children: [
          dialogTitleBar(t,
              title: l10n.mixerLoudnessTarget,
              onClose: widget.onCancel,
              keyPrefix: 'loudness-target'),
          Padding(
            padding: dialogBodyPadding,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                field('loudness-target-lufs', l10n.loudnessIntegrated, _lufs,
                    -40, -5, l10n.unitSymbolLufs, (v) => _lufs = v),
                field('loudness-target-peak', l10n.loudnessTruePeak, _peak, -9,
                    0, l10n.unitSymbolDbtp, (v) => _peak = v),
              ],
            ),
          ),
          dialogFooter(
            t,
            keyPrefix: 'loudness-target',
            actions: [
              HouseButton(
                key: const ValueKey('loudness-target-cancel'),
                onPressed: widget.onCancel,
                child: Text(l10n.cancel),
              ),
              HouseButton(
                key: const ValueKey('loudness-target-confirm'),
                primary: true,
                onPressed: _confirm,
                child: Text(l10n.apply),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

/// **Loudness target ▸** and **Normalise mix…**, as right-click menu rows.
List<Widget> loudnessMenuRows({
  required BuildContext context,
  required LumitState app,
  required CompositionReference comp,
  required BridgeLoudnessTarget? current,
  required VoidCallback close,
  required VoidCallback onChanged,
  // Room for a tick beside every name, where the menu's other rows have it.
  bool tickColumn = true,
}) =>
    [
      SubmenuRow(
        key: const ValueKey<String>('loudness-target'),
        closeParent: close,
        submenu: (dismiss) => FloatSurface(
          width: 140,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              MenuRow(
                key: const ValueKey<String>('loudness-target-off'),
                onPressed: () {
                  dismiss();
                  setCompLoudnessTarget(comp, null, onChanged);
                },
                child: MenuLabel(l10n.mixerLoudnessTargetOff,
                    ticked: current == null),
              ),
              for (final lufs in loudnessPresets)
                MenuRow(
                  key: ValueKey<String>('loudness-target-${-lufs.round()}'),
                  onPressed: () {
                    dismiss();
                    setCompLoudnessTarget(comp, presetTarget(lufs), onChanged);
                  },
                  child: MenuLabel(loudnessTargetLabel(lufs),
                      ticked: isPreset(current, lufs)),
                ),
              MenuRow(
                key: const ValueKey<String>('loudness-target-custom'),
                onPressed: () {
                  dismiss();
                  customLoudnessTarget(context, comp, onChanged);
                },
                child: MenuLabel(l10n.mixerLoudnessTargetCustom,
                    ticked: current != null &&
                        !loudnessPresets.any((l) => isPreset(current, l))),
              ),
            ],
          ),
        ),
        child: MenuLabel(l10n.mixerLoudnessTarget,
            ticked: tickColumn ? false : null),
      ),
      MenuRow(
        key: const ValueKey<String>('normalise-mix'),
        onPressed: () {
          close();
          normaliseMixCommand(context, app, comp, onChanged);
        },
        child: MenuLabel(l10n.mixerNormaliseMix,
            ticked: tickColumn ? false : null),
      ),
    ];
