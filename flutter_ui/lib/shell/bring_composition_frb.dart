// Bringing a composition out of another project file and into the open one.
//
// The file is an earlier save of the project, fetched from its history, or
// any `.lum` picked with Import footage. A small window lists the
// compositions it holds, and the one chosen comes in as a new composition
// with everything it places. The engine does the bringing in. This is the
// choosing.

import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/timecode.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

const double bringCompositionWidth = 460;
const double _rowHeight = 28;

/// The air over the first row and under the last.
const double _listInset = 6;

/// The list shows this many rows before it scrolls, and has a search box
/// when there are more.
const int _rowsShown = 8;

/// Let the person choose a composition from the project file at [path], and
/// bring it into the open project. True when one came in.
///
/// [earlierSave] says the file is a save of the open project, fetched from
/// its history. That changes the words, and has the engine look for the
/// file's footage from the project's own folder. [subject] is what the window
/// says it is choosing from.
///
/// [source] is what the file is called in a name. A composition whose name
/// the project already has arrives named for it as well, as in Titles
/// (Before the regrade), so the one brought in is never taken for the one
/// that was there.
Future<bool> bringCompositionFrom(BuildContext context, String path,
    {required bool earlierSave,
    String subject = '',
    String source = ''}) async {
  final app = context.read<LumitState>();
  final ui = context.read<LumitUiState>();
  final project = app.project;
  if (project == null) return false;
  final comps = await compsInFile(path: path);
  if (comps == null) {
    app.postNotice(l10n.bringUnreadable, error: true);
    return false;
  }
  if (comps.isEmpty) {
    app.postNotice(l10n.bringNoComps);
    return false;
  }
  if (!context.mounted) return false;
  final chosen = await showLumitModal<BridgeFileComp>(
    context: context,
    builder: (close) => _Picker(
        comps: comps, earlierSave: earlierSave, subject: subject, close: close),
  );
  if (chosen == null) return false;
  List<CompositionReference>? made;
  try {
    made = await project.bringComps(
        path: path,
        comps: [chosen.id],
        earlierSave: earlierSave,
        source: source);
  } catch (_) {
    // The project closed while the window was up. Said the same way.
  }
  if (made == null || made.isEmpty) {
    app.postNotice(l10n.bringUnreadable, error: true);
    return false;
  }
  app.notifyDocumentChanged();
  ui.projectPickRequest.value = ItemReference.composition(made.first);
  // The name it got, which is not the one it had when that was taken.
  var name = chosen.name;
  try {
    name = made.first.getSettings().name;
  } catch (_) {
    // The project closed as it came in.
  }
  app.postNotice(earlierSave
      ? l10n.backupBroughtBack(name)
      : l10n.importCompDone(name));
  return true;
}

class _Picker extends StatefulWidget {
  final List<BridgeFileComp> comps;
  final bool earlierSave;
  final String subject;
  final void Function(BridgeFileComp?) close;

  const _Picker(
      {required this.comps,
      required this.earlierSave,
      required this.subject,
      required this.close});

  @override
  State<_Picker> createState() => _PickerState();
}

class _PickerState extends State<_Picker> {
  final _search = TextEditingController();
  final _scroll = ScrollController();
  late BridgeFileComp _chosen = widget.comps.first;

  /// There is a search box when the list is long enough to scroll.
  bool get _searched => widget.comps.length > _rowsShown;

  /// The compositions the search leaves.
  List<BridgeFileComp> get _shown {
    final wanted = _search.text.trim().toLowerCase();
    return [
      for (final comp in widget.comps)
        if (comp.name.toLowerCase().contains(wanted)) comp,
    ];
  }

  @override
  void initState() {
    super.initState();
    _search.addListener(() => setState(() {}));
  }

  @override
  void dispose() {
    _search.dispose();
    _scroll.dispose();
    super.dispose();
  }

  /// Up and down move the choice and keep it in view. Home and End take it
  /// to the ends of a list with no search box over it. With one, they are
  /// the box's own keys.
  KeyEventResult _key(FocusNode node, KeyEvent event) {
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    final shown = _shown;
    if (shown.isEmpty) return KeyEventResult.ignored;
    final at = math.max(0, shown.indexOf(_chosen));
    final to = switch (event.logicalKey) {
      LogicalKeyboardKey.arrowUp => math.max(0, at - 1),
      LogicalKeyboardKey.arrowDown => math.min(shown.length - 1, at + 1),
      LogicalKeyboardKey.home when !_searched => 0,
      LogicalKeyboardKey.end when !_searched => shown.length - 1,
      _ => null,
    };
    if (to == null) return KeyEventResult.ignored;
    setState(() => _chosen = shown[to]);
    if (_scroll.hasClients) {
      final position = _scroll.position;
      final top = _listInset + to * _rowHeight;
      final seen = position.pixels + position.viewportDimension;
      if (top < position.pixels) {
        _scroll.jumpTo(top - _listInset);
      } else if (top + _rowHeight > seen) {
        _scroll.jumpTo(
            top + _rowHeight + _listInset - position.viewportDimension);
      }
    }
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final shown = _shown;
    // A search that hides the chosen one chooses the first it leaves.
    final chosen = shown.contains(_chosen) ? _chosen : shown.firstOrNull;
    final searched = _searched;
    final frame = DialogFrame(
      width: bringCompositionWidth,
      children: [
        dialogTitleBar(
          t,
          title: widget.earlierSave
              ? l10n.backupBringBackTitle
              : l10n.importCompTitle,
          subject: widget.subject,
          onClose: () => widget.close(null),
          keyPrefix: 'bring',
        ),
        if (searched)
          Padding(
            padding:
                const EdgeInsets.fromLTRB(dialogPadding, 10, dialogPadding, 2),
            child: SizedBox(
              height: dialogControlHeight,
              child: HouseTextField(
                key: const ValueKey('bring-search'),
                controller: _search,
                width: double.infinity,
                autofocus: true,
                hint: l10n.search,
                leading: glyph.LumitIcon(LumitIcons.search,
                    size: 12, colour: t.textMuted),
                onSubmitted: (_) {
                  if (chosen != null) widget.close(chosen);
                },
              ),
            ),
          ),
        SizedBox(
          // Whole rows, and the thumb down the side says when there are
          // more of them out of sight.
          height: _rowHeight * math.min(widget.comps.length, _rowsShown) +
              _listInset * 2,
          child: shown.isEmpty
              ? Center(
                  child: Text(l10n.noMatches,
                      style: t.body.copyWith(color: t.textSecondary)),
                )
              : dialogScroller(
                  context,
                  t,
                  _scroll,
                  child: ListView(
                    key: const ValueKey('bring-list'),
                    controller: _scroll,
                    padding: const EdgeInsets.symmetric(vertical: _listInset),
                    children: [
                      for (final comp in shown)
                        _CompRow(
                          key: ValueKey<String>('bring-comp-${comp.id}'),
                          comp: comp,
                          chosen: comp == chosen,
                          onChoose: () => setState(() => _chosen = comp),
                          onConfirm: () => widget.close(comp),
                        ),
                    ],
                  ),
                ),
        ),
        dialogFooter(
          t,
          keyPrefix: 'bring',
          actions: [
            HouseButton(
              key: const ValueKey('bring-cancel'),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              onPressed: () => widget.close(null),
              child: Text(l10n.cancel),
            ),
            dialogFilledAction(
              t,
              key: const ValueKey('bring-confirm'),
              // Enter presses it, where the search box has not got the keys.
              autofocus: !searched,
              label: widget.earlierSave
                  ? l10n.backupBringBackConfirm
                  : l10n.importCompConfirm,
              onPressed: chosen == null ? null : () => widget.close(chosen),
            ),
          ],
        ),
      ],
    );
    return Focus(onKeyEvent: _key, child: frame);
  }
}

/// One composition in the list: its name, its size and how long it is.
class _CompRow extends StatefulWidget {
  final BridgeFileComp comp;
  final bool chosen;
  final VoidCallback onChoose, onConfirm;

  const _CompRow(
      {super.key,
      required this.comp,
      required this.chosen,
      required this.onChoose,
      required this.onConfirm});

  @override
  State<_CompRow> createState() => _CompRowState();
}

class _CompRowState extends State<_CompRow> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final comp = widget.comp;
    final chosen = widget.chosen;
    Widget reading(String text, double width) => SizedBox(
          width: width,
          child: Text(text,
              textAlign: TextAlign.right,
              style: dialogMono(t).copyWith(
                  color: chosen ? t.textSecondary : t.textMuted)),
        );
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        // Chosen as the pointer goes down, so the second press of a double
        // click is not waited for before the row lights.
        onTapDown: (_) => widget.onChoose(),
        onDoubleTap: widget.onConfirm,
        child: Container(
          height: _rowHeight,
          padding: const EdgeInsets.symmetric(horizontal: dialogPadding),
          color: chosen
              ? t.selectionFill
              : _hover
                  ? t.surface2
                  : null,
          // The edge the chosen version wears in the history it came from.
          foregroundDecoration: chosen
              ? BoxDecoration(
                  border: Border(left: BorderSide(color: t.accent, width: 2)),
                )
              : null,
          child: Row(
            children: [
              glyph.LumitIcon(LumitIcons.composition,
                  size: 13, colour: chosen ? t.textPrimary : t.textMuted),
              const SizedBox(width: 9),
              Expanded(
                child: Text(
                  comp.name,
                  style: chosen ? t.bodyPrimary : t.body,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              reading('${comp.width}×${comp.height}', 84),
              reading(
                  timecodeOfRate(
                      comp.durationFrames.toInt(), comp.fpsNum, comp.fpsDen),
                  92),
            ],
          ),
        ),
      ),
    );
  }
}
