// The Pre-compose dialogue (Ctrl+Shift+C, or Layer ▸ Pre-compose…).
//
// Packing layers into a comp of their own is one engine call, but it asks two
// questions first that only the user can answer (docs/07 §13.4): whether the
// attributes travel with the layer or stay behind on the Precomp layer, and
// whether the new comp is as long as this one or only as long as the selection.
// Both answers are remembered in the workspace, because a person who works one
// way tends to keep working that way.
//
// Leaving the attributes behind only means anything for a single layer — there
// is no one layer for a stack's transforms to stay on — so with more than one
// selected the choice is disabled and Move is the answer. The engine refuses
// the impossible combination too; this is the same rule said early enough to
// keep the dialogue honest.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../main.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

/// Show the dialogue and, on confirm, precompose. Returns when it closes.
///
/// [group] names the layer group being packed when the command came from a
/// group header's *Pre-compose group*: the engine moves the header's
/// effect stack onto the new Precomp layer and takes the emptied band away in
/// the same undo step.
Future<void> showPrecomposeDialogFrb({
  required BuildContext context,
  required CompositionReference comp,
  required List<LayerReference> selectedLayers,
  required LumitUiState ui,
  required Workspace workspace,
  UuidValue? group,
}) async {
  if (selectedLayers.isEmpty) return;

  // The same layer twice is a selection quirk, not two layers: it would make
  // the engine refuse a single-layer Leave, and pack a duplicate on Move.
  final seen = <UuidValue>{};
  final layers = [
    for (final l in selectedLayers)
      if (seen.add(l.internallayerId)) l,
  ];

  await showLumitModal<void>(
    context: context,
    id: 'precompose',
    builder: (close) => _PrecomposeBody(
      parentCompName: comp.getSettings().name,
      selectedCount: layers.length,
      defaultName: '${layers.first.getName()} Comp',
      initialMoveAttributes: workspace.precomposeMoveAttributes,
      initialAdjustDuration: workspace.precomposeAdjustDuration,
      initialOpenNewComp: workspace.precomposeOpenNewComp,
      onConfirm: (name, moveAttributes, adjustDuration, openNewComp) async {
        final ids = [for (final l in layers) l.internallayerId];
        final leave = !moveAttributes && layers.length == 1;
        // A parent or matte that would cross the new comp's edge: the engine
        // keeps it by duplicating a layer, which is asked about first.
        var splits = false;
        try {
          splits =
              comp.precomposeSplitsLinks(layerIds: ids, leaveAttributes: leave);
        } catch (_) {}
        if (splits && context.mounted) {
          final go = await askLumitChoice<bool>(
            context: context,
            title: l10n.precomposeLinksTitle,
            question: l10n.precomposeLinksQuestion,
            answers: [(l10n.precomposeDuplicate, true)],
          );
          if (go != true) return null;
        }
        workspace.setPrecomposeSettings(
          moveAttributes: moveAttributes,
          adjustDuration: adjustDuration,
          openNewComp: openNewComp,
        );
        final LayerReference precomp;
        try {
          precomp = comp.precompose(
            layerIds: ids,
            name: name,
            leaveAttributes: leave,
            adjustDuration: adjustDuration,
            group: group,
          );
        } catch (_) {
          // The dialogue stays open saying so, rather than closing on a move
          // that never happened.
          return l10n.precomposeFailed;
        }
        // The Precomp layer is what the user is now working on.
        ui.setSelection([precomp]);
        ui.model.refresh();
        if (openNewComp) {
          if (precomp.getSourceItem()
              case ItemReference_Composition(
                :final field0,
              )) {
            ui.setSelectedComp(field0);
          }
        }
        close(null);
        return null;
      },
      onCancel: () => close(null),
    ),
  );
}

/// `onConfirm` returns null when the move went through or was called off, or
/// the sentence to show when it did not — the dialogue stays open on a
/// refusal.
typedef _Confirm = Future<String?> Function(
  String name,
  bool moveAttributes,
  bool adjustDuration,
  bool openNewComp,
);

class _PrecomposeBody extends StatefulWidget {
  final String parentCompName;
  final int selectedCount;
  final String defaultName;
  final bool initialMoveAttributes;
  final bool initialAdjustDuration;
  final bool initialOpenNewComp;
  final _Confirm onConfirm;
  final VoidCallback onCancel;

  const _PrecomposeBody({
    required this.parentCompName,
    required this.selectedCount,
    required this.defaultName,
    required this.initialMoveAttributes,
    required this.initialAdjustDuration,
    required this.initialOpenNewComp,
    required this.onConfirm,
    required this.onCancel,
  });

  @override
  State<_PrecomposeBody> createState() => _PrecomposeBodyState();
}

class _PrecomposeBodyState extends State<_PrecomposeBody> {
  late final TextEditingController _name;
  late bool _moveAttributes;
  late bool _adjustDuration;
  late bool _openNewComp;
  String? _refusal;

  bool get _single => widget.selectedCount == 1;

  @override
  void initState() {
    super.initState();
    _name = TextEditingController(text: widget.defaultName);
    // Remembered, except where the selection makes the remembered answer
    // impossible: a stack always moves.
    _moveAttributes = _single ? widget.initialMoveAttributes : true;
    _adjustDuration = widget.initialAdjustDuration;
    _openNewComp = widget.initialOpenNewComp;
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _confirm() async {
    final typed = _name.text.trim();
    final refusal = await widget.onConfirm(
      typed.isEmpty ? widget.defaultName : typed,
      _moveAttributes,
      _adjustDuration,
      _openNewComp,
    );
    if (refusal != null && mounted) setState(() => _refusal = refusal);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return dialogEnter(
      onEnter: _confirm,
      child: DialogFrame(
        width: 420,
        children: [
          dialogTitleBar(t,
              title: l10n.precompose,
              onClose: widget.onCancel,
              keyPrefix: 'precompose'),
          Padding(
            padding: dialogBodyPadding,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                dialogRow(
                  t,
                  l10n.name,
                  HouseTextField(
                    key: const ValueKey('precompose-name'),
                    controller: _name,
                    width: double.infinity,
                    onSubmitted: (_) => _confirm(),
                  ),
                  labelColumn: 60,
                ),
                const SizedBox(height: 8),
                _choice(
                  t,
                  key: 'precompose-leave',
                  selected: !_moveAttributes,
                  enabled: _single,
                  onPick: () => setState(() => _moveAttributes = false),
                  label: l10n.precomposeLeave(widget.parentCompName),
                ),
                const SizedBox(height: 8),
                _choice(
                  t,
                  key: 'precompose-move',
                  selected: _moveAttributes,
                  enabled: true,
                  onPick: () => setState(() => _moveAttributes = true),
                  label: l10n.precomposeMove,
                ),
                const SizedBox(height: 12),
                _check(
                  t,
                  key: 'precompose-adjust-duration',
                  value: _adjustDuration,
                  onChanged: (v) => setState(() => _adjustDuration = v),
                  label: l10n.precomposeAdjustDuration,
                ),
                const SizedBox(height: 8),
                _check(
                  t,
                  key: 'precompose-open-new-comp',
                  value: _openNewComp,
                  onChanged: (v) => setState(() => _openNewComp = v),
                  label: l10n.precomposeOpenNew,
                ),
              ],
            ),
          ),
          dialogFooter(
            t,
            keyPrefix: 'precompose',
            summary: _refusal ?? '',
            actions: [
              HouseButton(
                key: const ValueKey('precompose-cancel'),
                onPressed: widget.onCancel,
                child: Text(l10n.cancel),
              ),
              HouseButton(
                key: const ValueKey('precompose-confirm'),
                primary: true,
                onPressed: _confirm,
                child: Text(l10n.precompose),
              ),
            ],
          ),
        ],
      ),
    );
  }

  /// One radio choice: the button and its sentence. The whole row is the
  /// target, so the sentence is as clickable as the dot.
  Widget _choice(
    LumitTheme t, {
    required String key,
    required bool selected,
    required bool enabled,
    required VoidCallback onPick,
    required String label,
  }) =>
      HoverMark(
        cursor: enabled ? SystemMouseCursors.click : MouseCursor.defer,
        builder: (context, _) => GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: enabled ? onPick : null,
          child: Row(
            children: [
              HouseRadio(
                key: ValueKey(key),
                selected: selected,
                enabled: enabled,
                onChanged: onPick,
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  label,
                  style: t.body.copyWith(
                    color: enabled ? t.textPrimary : t.textDisabled,
                  ),
                ),
              ),
            ],
          ),
        ),
      );

  Widget _check(
    LumitTheme t, {
    required String key,
    required bool value,
    required ValueChanged<bool> onChanged,
    required String label,
  }) =>
      HoverMark(
        builder: (context, lit) => GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: () => onChanged(!value),
          child: Row(
            children: [
              HouseCheckbox(
                key: ValueKey(key),
                value: value,
                onChanged: onChanged,
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Text(label,
                    style: t.body.copyWith(
                        color: Color.lerp(t.body.color, t.textPrimary, lit))),
              ),
            ],
          ),
        ),
      );
}
