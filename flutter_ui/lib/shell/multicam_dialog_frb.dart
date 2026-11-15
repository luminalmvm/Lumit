// The New multicam comp window.
//
// A name, how the files are lined up, what makes an angle, and Make. The
// engine does the work on a thread of its own, so while it runs the footer's
// buttons give way to a progress bar with Cancel beside it, and the window
// closes on its own when the comp lands.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/multicam/make.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../main.dart';
import '../panels/timeline_extras_frb.dart' show Segment, SegmentOption;
import '../state/dock.dart' show WorkspacePreset;
import '../state/multicam.dart' show MulticamLanded;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

const double _width = 420;
const double _labelColumn = 90;

/// The Composition menu's row: null, so it greys out, while the Project panel
/// has no footage selected.
VoidCallback? newMulticamAction(BuildContext context, LumitUiState ui) {
  final footage = [
    for (final item in ui.selectedProjectItems.value)
      if (item case ItemReference_Footage(:final field0)) field0,
  ];
  if (footage.isEmpty) return null;
  return () => showMulticamDialogFrb(context: context, items: footage);
}

/// Open the window on [items], in the order they are given. When the comp
/// lands it is selected in the Project panel, and its angles go to the source
/// side when the Cut workspace is up.
Future<void> showMulticamDialogFrb({
  required BuildContext context,
  required List<FootageReference> items,
}) async {
  if (items.isEmpty) return;
  final ui = context.read<LumitUiState>();
  final app = context.read<LumitState>();
  final project = app.project;
  // One job at a time, for the whole process.
  if (project == null || ui.multicam.working.value != null) return;
  final String first;
  try {
    first = ItemReference.footage(items.first).name();
  } catch (_) {
    return;
  }
  final made = await showLumitModal<UuidValue>(
    context: context,
    id: 'multicam-new',
    builder: (close) => _MulticamBody(
      ui: ui,
      items: items,
      // Named for the first file, without its extension.
      name: l10n.multicamDefaultName(first.lastIndexOf('.') > 0
          ? first.substring(0, first.lastIndexOf('.'))
          : first),
      onMade: close,
      onCancel: () => close(null),
    ),
  );
  if (made == null) return;
  final comp =
      CompositionReference(internalproject: project.internalid, internalid: made);
  ui.projectPickRequest.value = ItemReference.composition(comp);
  if (ui.workspace.activePreset == WorkspacePreset.cut) ui.showAngles(comp);
}

class _MulticamBody extends StatefulWidget {
  final LumitUiState ui;
  final List<FootageReference> items;
  final String name;
  final void Function(UuidValue comp) onMade;
  final VoidCallback onCancel;

  const _MulticamBody({
    required this.ui,
    required this.items,
    required this.name,
    required this.onMade,
    required this.onCancel,
  });

  @override
  State<_MulticamBody> createState() => _MulticamBodyState();
}

class _MulticamBodyState extends State<_MulticamBody> {
  late final TextEditingController _name =
      TextEditingController(text: widget.name);
  BridgeMulticamSync _sync = BridgeMulticamSync.sound;
  BridgeMulticamAngles _angles = BridgeMulticamAngles.eachFile;
  bool _drift = true;

  /// Whoever was told of landings before this window, told again after it.
  MulticamLanded? _wasLanded;

  bool get _busy => widget.ui.multicam.working.value != null;

  @override
  void initState() {
    super.initState();
    widget.ui.multicam.working.addListener(_changed);
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  void _make() {
    if (_busy) return;
    final mc = widget.ui.multicam;
    _wasLanded = mc.onLanded;
    mc.onLanded = (comp, {required made}) {
      if (made && mounted) widget.onMade(comp);
    };
    mc.shown = true;
    final started = mc.make(
      [for (final item in widget.items) item.internalid],
      BridgeMulticamNew(
        name: _name.text.trim(),
        sync_: _sync,
        angles: _angles,
        drift: _sync == BridgeMulticamSync.sound && _drift,
      ),
    );
    if (!started) _release();
  }

  /// Hand the controller back as it was found.
  void _release() {
    final mc = widget.ui.multicam;
    if (!mc.shown) return;
    mc.shown = false;
    mc.onLanded = _wasLanded;
    _wasLanded = null;
  }

  @override
  void dispose() {
    final mc = widget.ui.multicam;
    mc.working.removeListener(_changed);
    // Closed while the job ran, by Escape or the close mark: nothing is made.
    if (mc.shown && _busy) mc.cancel();
    _release();
    _name.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final busy = _busy;
    return dialogEnter(
      onEnter: _make,
      child: DialogFrame(
        width: _width,
        children: [
          dialogTitleBar(
            t,
            title: l10n.multicamDialogTitle,
            onClose: widget.onCancel,
            keyPrefix: 'multicam',
          ),
          IgnorePointer(
            ignoring: busy,
            child: Padding(
              padding: dialogBodyPadding,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  _row(
                    t,
                    l10n.name,
                    SizedBox(
                      height: dialogControlHeight,
                      child: HouseTextField(
                        key: const ValueKey('multicam-name'),
                        controller: _name,
                        width: double.infinity,
                        padding: const EdgeInsets.symmetric(horizontal: 8),
                        fill: t.surface0,
                        onSubmitted: (_) => _make(),
                      ),
                    ),
                    busy,
                  ),
                  _row(
                    t,
                    l10n.multicamSyncBy,
                    Align(
                      alignment: AlignmentDirectional.centerStart,
                      child: dialogDropdown<BridgeMulticamSync>(
                        t,
                        id: 'multicam-sync',
                        value: _sync,
                        options: BridgeMulticamSync.values,
                        label: (v) => switch (v) {
                          BridgeMulticamSync.sound => l10n.multicamSyncSound,
                          BridgeMulticamSync.timecode =>
                            l10n.multicamSyncTimecode,
                          BridgeMulticamSync.inMarks => l10n.multicamSyncInMarks,
                          BridgeMulticamSync.outMarks =>
                            l10n.multicamSyncOutMarks,
                          BridgeMulticamSync.markers => l10n.multicamSyncMarkers,
                          BridgeMulticamSync.starts => l10n.multicamSyncStarts,
                        },
                        onChanged: (v) => setState(() => _sync = v),
                        width: 160,
                      ),
                    ),
                    busy,
                  ),
                  _row(
                    t,
                    l10n.multicamAngles,
                    _segment(t, 'angles', _angles, {
                      BridgeMulticamAngles.eachFile: l10n.multicamEachFile,
                      BridgeMulticamAngles.eachFolder: l10n.multicamEachFolder,
                    }, (v) => setState(() => _angles = v)),
                    busy,
                  ),
                  if (_sync == BridgeMulticamSync.sound)
                    _row(
                      t,
                      l10n.multicamDrift,
                      Align(
                        alignment: AlignmentDirectional.centerStart,
                        child: HouseCheckbox(
                          key: const ValueKey('multicam-drift'),
                          value: _drift,
                          onChanged: busy
                              ? null
                              : (v) => setState(() => _drift = v),
                        ),
                      ),
                      busy,
                    ),
                ],
              ),
            ),
          ),
          dialogFooter(
            t,
            keyPrefix: 'multicam',
            leading: busy
                ? SizedBox(
                    width: 160,
                    child: Center(
                      child: HouseProgressBar(
                        key: const ValueKey('multicam-progress'),
                        fraction: widget.ui.multicam.working.value ?? 0,
                      ),
                    ),
                  )
                : null,
            actions: [
              HouseButton(
                key: const ValueKey('multicam-cancel'),
                padding: const EdgeInsets.symmetric(horizontal: 12),
                onPressed:
                    busy ? widget.ui.multicam.cancel : widget.onCancel,
                child: Text(l10n.cancel),
              ),
              if (!busy)
                HouseButton(
                  key: const ValueKey('multicam-make'),
                  primary: true,
                  padding: const EdgeInsets.symmetric(horizontal: 16),
                  onPressed: _make,
                  child: Text(l10n.multicamMake),
                ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _row(LumitTheme t, String label, Widget control, bool dimmed) =>
      dialogRow(t, label, control,
          labelColumn: _labelColumn, dimmed: dimmed);

  /// The house pill, one option in force.
  Widget _segment<T>(LumitTheme t, String key, T current,
          Map<T, String> options, ValueChanged<T> onPick) =>
      Align(
        alignment: AlignmentDirectional.centerStart,
        child: Segment(children: [
          for (final MapEntry(key: value, value: label) in options.entries)
            SegmentOption(
              key: ValueKey('multicam-$key-${value.toString().split('.').last}'),
              active: value == current,
              onTap: () => onPick(value),
              child: Text(label, style: t.small),
            ),
        ]),
      );
}
