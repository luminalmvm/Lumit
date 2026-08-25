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

import 'package:flutter/services.dart';
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

/// Show the dialogue and, on confirm, precompose. Returns when it closes.
Future<void> showPrecomposeDialogFrb({
  required BuildContext context,
  required CompositionReference comp,
  required List<LayerReference> selectedLayers,
  required LumitUiState ui,
  required Workspace workspace,
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
      layerName: layers.first.getName(),
      selectedCount: layers.length,
      defaultName: '${layers.first.getName()} Comp',
      initialMoveAttributes: workspace.precomposeMoveAttributes,
      initialAdjustDuration: workspace.precomposeAdjustDuration,
      initialOpenNewComp: workspace.precomposeOpenNewComp,
      onConfirm: (name, moveAttributes, adjustDuration, openNewComp) {
        workspace.setPrecomposeSettings(
          moveAttributes: moveAttributes,
          adjustDuration: adjustDuration,
          openNewComp: openNewComp,
        );
        final LayerReference precomp;
        try {
          precomp = comp.precompose(
            layerIds: [for (final l in layers) l.internallayerId],
            name: name,
            leaveAttributes: !moveAttributes && layers.length == 1,
            adjustDuration: adjustDuration,
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

/// `onConfirm` returns null when the move went through, or the sentence to
/// show when it did not — the dialogue stays open on a refusal.
typedef _Confirm = String? Function(
  String name,
  bool moveAttributes,
  bool adjustDuration,
  bool openNewComp,
);

class _PrecomposeBody extends StatefulWidget {
  final String parentCompName;
  final String layerName;
  final int selectedCount;
  final String defaultName;
  final bool initialMoveAttributes;
  final bool initialAdjustDuration;
  final bool initialOpenNewComp;
  final _Confirm onConfirm;
  final VoidCallback onCancel;

  const _PrecomposeBody({
    required this.parentCompName,
    required this.layerName,
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

  void _confirm() {
    final typed = _name.text.trim();
    final refusal = widget.onConfirm(
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
    // The dialogue takes focus when it opens, and `Enter` is Pre-compose
    // wherever that focus sits (K-243) — the button at the bottom is the
    // default action, so the keyboard should be able to say yes without first
    // having to find it. Typing a name is one click into the field.
    return Focus(
      autofocus: true,
      onKeyEvent: (_, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        if (event.logicalKey != LogicalKeyboardKey.enter &&
            event.logicalKey != LogicalKeyboardKey.numpadEnter) {
          return KeyEventResult.ignored;
        }
        _confirm();
        return KeyEventResult.handled;
      },
      child: _body(t),
    );
  }

  Widget _body(LumitTheme t) {
    return FloatSurface(
      width: 440,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Text(
              l10n.precompose,
              style: t.bodyPrimary,
              textAlign: TextAlign.center,
            ),
          ),
          // The label and its field share one line: the name is short, and a
          // wrapped label beside a box reads as two questions rather than one.
          Row(
            children: [
              Text(l10n.precomposeNewName, style: t.small, softWrap: false),
              const SizedBox(width: 8),
              Expanded(
                child: HouseTextField(
                  key: const ValueKey('precompose-name'),
                  controller: _name,
                  width: double.infinity,
                  onSubmitted: (_) => _confirm(),
                ),
              ),
            ],
          ),
          const SizedBox(height: 14),
          _choice(
            t,
            key: 'precompose-leave',
            selected: !_moveAttributes,
            enabled: _single,
            onPick: () => setState(() => _moveAttributes = false),
            label: l10n.precomposeLeave(widget.parentCompName),
            caption: l10n.precomposeLeaveHelp(widget.layerName),
          ),
          const SizedBox(height: 12),
          _choice(
            t,
            key: 'precompose-move',
            selected: _moveAttributes,
            enabled: true,
            onPick: () => setState(() => _moveAttributes = true),
            label: l10n.precomposeMove,
            caption: l10n.precomposeMoveHelp,
          ),
          const SizedBox(height: 14),
          // Indented under the choices above: it qualifies the new composition
          // they make, rather than standing beside them as a third choice.
          Padding(
            padding: const EdgeInsets.only(left: 22),
            child: _check(
              t,
              key: 'precompose-adjust-duration',
              value: _adjustDuration,
              onChanged: (v) => setState(() => _adjustDuration = v),
              label: l10n.precomposeAdjustDuration,
            ),
          ),
          const SizedBox(height: 8),
          _check(
            t,
            key: 'precompose-open-new-comp',
            value: _openNewComp,
            onChanged: (v) => setState(() => _openNewComp = v),
            label: l10n.precomposeOpenNew,
          ),
          if (_refusal != null)
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: Text(
                _refusal!,
                key: const ValueKey('precompose-refusal'),
                style: t.small.copyWith(color: t.textMuted),
              ),
            ),
          const SizedBox(height: 16),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              HouseButton(
                key: const ValueKey('precompose-confirm'),
                primary: true,
                onPressed: _confirm,
                child: Text(l10n.precompose),
              ),
              const SizedBox(width: 8),
              HouseButton(
                key: const ValueKey('precompose-cancel'),
                onPressed: widget.onCancel,
                child: Text(l10n.cancel),
              ),
            ],
          ),
        ],
      ),
    );
  }

  /// One radio choice: the button, its sentence, and the explanation under it.
  /// The whole block is the target, so the sentence is as clickable as the dot.
  Widget _choice(
    LumitTheme t, {
    required String key,
    required bool selected,
    required bool enabled,
    required VoidCallback onPick,
    required String label,
    required String caption,
  }) =>
      GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: enabled ? onPick : null,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
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
                    style: t.small.copyWith(
                      color: enabled ? t.textPrimary : t.textMuted,
                    ),
                  ),
                ),
              ],
            ),
            Padding(
              padding: const EdgeInsets.only(left: 22, top: 4),
              child: Text(
                caption,
                style: t.caption.copyWith(
                  height: 1.3,
                  // A dimmed explanation under a dimmed choice: it describes
                  // something that is not on offer for this selection.
                  color: enabled
                      ? t.textMuted
                      : t.textMuted.withValues(alpha: 0.5),
                ),
              ),
            ),
          ],
        ),
      );

  Widget _check(
    LumitTheme t, {
    required String key,
    required bool value,
    required ValueChanged<bool> onChanged,
    required String label,
  }) =>
      GestureDetector(
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
            Expanded(child: Text(label, style: t.small)),
          ],
        ),
      );
}
