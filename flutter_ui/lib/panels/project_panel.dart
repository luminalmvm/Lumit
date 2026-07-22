// The live Project panel (phase F1 + the interactive slice): one row per
// document item, folders nesting their children. A click selects (highlight); a
// second click on the selected row renames it in place (→ `renameItem`); a
// double-click opens a composition (fronts it) or places a footage item into the
// front comp as a new layer; a right-click raises the egui project menu
// (Composition settings / Relink / Find missing footage / Move to root / Delete);
// and a footage row is Draggable onto the Timeline lane. A missing footage row
// wears a "missing" badge with an inline Relink… button (docs/07 §3.3), and a
// "Show only missing footage" toggle appears in the header when anything IS
// missing. An empty document shows a quiet hint.
//
// Thumbnails (docs/06 §5): a footage row shows a small decoded thumbnail of the
// item (`app.requestThumbnail`, the `ThumbnailBridge` binding via the render
// worker isolate) in place of its type glyph, decoded off the UI isolate and
// cached until the document epoch advances (a relink re-decodes). A placeholder
// glyph shows until the picture lands, and the glyph stays for a missing file
// or a build without the capability.

import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';

import '../bridge/bridge.dart';
import '../icons/icons.dart';
import '../shell/dialogs.dart';
import '../state/app_state.dart';
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

class ProjectPanel extends StatefulWidget {
  final AppStateStub app;

  /// The relink file picker seam (path chosen → its path, or null when
  /// cancelled). Defaults to the real footage picker's single-file variant;
  /// tests inject their own so no plugin channel opens.
  final Future<String?> Function()? relinkPicker;

  const ProjectPanel({super.key, required this.app, this.relinkPicker});

  @override
  State<ProjectPanel> createState() => _ProjectPanelState();
}

class _ProjectPanelState extends State<ProjectPanel> {
  bool _missingOnly = false;

  AppStateStub get app => widget.app;

  Future<String?> _relink() async {
    final picker = widget.relinkPicker;
    if (picker != null) return picker();
    final paths = await pickFootage();
    return paths.isEmpty ? null : paths.first;
  }

  /// Every footage item id whose file is missing, across the snapshot tree.
  Set<String> _missingIds(List<BridgeItem> items) {
    final out = <String>{};
    void walk(BridgeItem item) {
      if (item.kind == BridgeItemKind.footage &&
          item.status == BridgeMediaStatus.missing) {
        out.add(item.id);
      }
      for (final c in item.children) {
        walk(c);
      }
    }

    for (final item in items) {
      walk(item);
    }
    return out;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ListenableBuilder(
      listenable: app,
      builder: (context, _) {
        final snapshot = app.snapshot;
        final items = snapshot?.items ?? const <BridgeItem>[];
        if (items.isEmpty) {
          return Center(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 240),
              child: Text(
                'No items yet — import footage or create a composition',
                style: t.small,
                textAlign: TextAlign.center,
              ),
            ),
          );
        }
        final missing = _missingIds(items);
        // The filter only bites while something is missing; a healthy project
        // never traps the user behind an empty "missing only" view.
        final missingOnly = _missingOnly && missing.isNotEmpty;

        final rows = <Widget>[];
        void walk(BridgeItem item, int depth) {
          // In missing-only mode, show only the missing footage rows (every
          // visible row is then something to fix, docs/07 §3.3).
          final show = !missingOnly ||
              (item.kind == BridgeItemKind.footage && missing.contains(item.id));
          if (show) {
            rows.add(_ProjectRow(
              key: ValueKey<String>('project-row-${item.id}'),
              app: app,
              item: item,
              depth: depth,
              missing: missing.contains(item.id),
              relink: _relink,
              onFindMissing: () => setState(() => _missingOnly = true),
            ));
          }
          for (final child in item.children) {
            walk(child, depth + 1);
          }
        }

        for (final item in items) {
          walk(item, 0);
        }
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (missing.isNotEmpty)
              _MissingHeader(
                count: missing.length,
                active: missingOnly,
                onToggle: () => setState(() => _missingOnly = !_missingOnly),
              ),
            Expanded(
              child: ListView(
                padding: const EdgeInsets.symmetric(vertical: 4),
                children: rows,
              ),
            ),
          ],
        );
      },
    );
  }
}

/// The header strip shown when the project has missing footage: a count and a
/// "show only missing" toggle (the egui missing-only control, panels.rs:176).
class _MissingHeader extends StatelessWidget {
  final int count;
  final bool active;
  final VoidCallback onToggle;
  const _MissingHeader({
    required this.count,
    required this.active,
    required this.onToggle,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: active
          ? 'Showing only missing footage — click to show everything'
          : 'Show only missing footage',
      child: GestureDetector(
        key: const ValueKey('missing-toggle'),
        behavior: HitTestBehavior.opaque,
        onTap: onToggle,
        child: Container(
          height: 24,
          color: active ? t.accent.withValues(alpha: 0.12) : t.surface1,
          padding: const EdgeInsets.symmetric(horizontal: 8),
          child: Row(
            children: [
              lumitIcon(LumitIcon.unlink, size: 13, color: t.warning),
              const SizedBox(width: 6),
              Text(
                '$count missing file${count == 1 ? '' : 's'}',
                style: t.small.copyWith(color: t.warning),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// One Project panel row: a type icon (tinted with the layer colours where it
/// reads well), the item name, indented 14 px per level. Selected rows carry the
/// `surface2` highlight, hovered rows the `surface4` fill. A composition
/// double-click fronts it; a footage double-click (or a drag onto the Timeline)
/// places it as a layer; a second click on the selected row renames it in place;
/// a right-click raises the project context menu.
class _ProjectRow extends StatefulWidget {
  final AppStateStub app;
  final BridgeItem item;
  final int depth;
  final bool missing;
  final Future<String?> Function() relink;
  final VoidCallback onFindMissing;
  const _ProjectRow({
    super.key,
    required this.app,
    required this.item,
    required this.depth,
    required this.missing,
    required this.relink,
    required this.onFindMissing,
  });

  @override
  State<_ProjectRow> createState() => _ProjectRowState();
}

class _ProjectRowState extends State<_ProjectRow> {
  bool _hover = false;
  bool _renaming = false;
  TextEditingController? _rename;
  final FocusNode _renameFocus = FocusNode();

  AppStateStub get app => widget.app;
  BridgeItem get item => widget.item;

  @override
  void dispose() {
    _rename?.dispose();
    _renameFocus.dispose();
    super.dispose();
  }

  void _startRename() {
    setState(() {
      _renaming = true;
      _rename = TextEditingController(text: item.name);
    });
    _renameFocus.requestFocus();
  }

  void _commitRename() {
    final text = _rename?.text.trim() ?? '';
    if (text.isNotEmpty && text != item.name) {
      app.renameItem(item.id, text);
    }
    setState(() {
      _renaming = false;
      _rename?.dispose();
      _rename = null;
    });
  }

  void _handleTap() {
    // A second click on the already-selected row starts an in-place rename
    // (the AE click-to-rename-when-selected gesture).
    if (app.selectedProjectItem == item.id && !_renaming) {
      _startRename();
    } else {
      app.selectProjectItem(item.id);
    }
  }

  void _handleDoubleTap() {
    switch (item.kind) {
      case BridgeItemKind.composition:
        app.frontCompSelect(item.id);
      case BridgeItemKind.footage:
        app.addFootageToFrontComp(item.id);
      case BridgeItemKind.folder:
      case BridgeItemKind.solid:
      case BridgeItemKind.unknown:
        break;
    }
  }

  Future<void> _doRelink() async {
    final path = await widget.relink();
    if (path != null) app.relink(item.id, path);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final (icon, tint) = _iconFor(item.kind, t);
    final selected = app.selectedProjectItem == item.id;
    final row = MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: _handleTap,
        onDoubleTap: _handleDoubleTap,
        onSecondaryTapDown: (d) {
          app.selectProjectItem(item.id);
          showProjectContextMenu(
            context: context,
            app: app,
            item: item,
            missing: widget.missing,
            position: d.globalPosition,
            onFindMissing: widget.onFindMissing,
            onRelink: _doRelink,
          );
        },
        child: Container(
          constraints: const BoxConstraints(minHeight: 22),
          color: selected
              ? t.surface2
              : _hover
                  ? t.surface4
                  : null,
          padding: EdgeInsets.only(left: 6.0 + widget.depth * 14.0, right: 6),
          child: Row(
            children: [
              _leading(t, icon, tint),
              const SizedBox(width: 6),
              Expanded(child: _nameOrEditor(t)),
              if (widget.missing) ...[
                const SizedBox(width: 6),
                Text('missing',
                    style: t.small.copyWith(color: t.warning)),
                const SizedBox(width: 6),
                LumitTooltip(
                  message: 'Relink this file to its new location',
                  child: HouseButton(
                    key: ValueKey<String>('relink-${item.id}'),
                    small: true,
                    onPressed: _doRelink,
                    child: Text('Relink…', style: t.small),
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
    if (item.kind == BridgeItemKind.footage) {
      return Draggable<FootageDragData>(
        data: FootageDragData(item.id, item.name),
        dragAnchorStrategy: pointerDragAnchorStrategy,
        feedback: _DragFeedback(name: item.name),
        child: row,
      );
    }
    return row;
  }

  /// The leading cell: a decoded thumbnail for a present footage item (when the
  /// bridge offers thumbnails), else the type glyph. Missing footage keeps the
  /// warning-tinted unlink glyph.
  Widget _leading(LumitTheme t, LumitIcon icon, Color tint) {
    final glyph = lumitIcon(
      widget.missing ? LumitIcon.unlink : icon,
      size: 14,
      color: widget.missing ? t.warning : tint,
    );
    if (item.kind != BridgeItemKind.footage ||
        widget.missing ||
        app.thumbnails == null) {
      return glyph;
    }
    return _FootageThumbnail(
      key: ValueKey<String>('thumb-${item.id}'),
      app: app,
      itemId: item.id,
      epoch: app.documentEpoch,
      placeholder: glyph,
    );
  }

  Widget _nameOrEditor(LumitTheme t) {
    if (_renaming && _rename != null) {
      return Container(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
        decoration: BoxDecoration(
          color: t.surface0,
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          border: Border.all(color: t.accent),
        ),
        child: EditableText(
          key: const ValueKey('rename-field'),
          controller: _rename!,
          focusNode: _renameFocus,
          style: t.body,
          cursorColor: t.accent,
          backgroundCursorColor: t.surface2,
          selectionColor: t.accent.withValues(alpha: 0.5),
          onSubmitted: (_) => _commitRename(),
          onTapOutside: (_) => _commitRename(),
        ),
      );
    }
    return Text(item.name, style: t.body, overflow: TextOverflow.ellipsis);
  }

  (LumitIcon, Color) _iconFor(BridgeItemKind kind, LumitTheme t) =>
      switch (kind) {
        BridgeItemKind.footage => (LumitIcon.footage, t.layer.footage),
        BridgeItemKind.folder => (LumitIcon.folder, t.textMuted),
        BridgeItemKind.composition => (LumitIcon.comp, t.layer.precomp),
        BridgeItemKind.solid => (LumitIcon.solid, t.layer.solid),
        BridgeItemKind.unknown => (LumitIcon.footage, t.textMuted),
      };
}

/// A footage row's thumbnail: decoded asynchronously through
/// `app.requestThumbnail` (on the render worker isolate; once, cached
/// engine-side) and held as a `ui.Image` until the document epoch
/// advances. Shows [placeholder] until the picture lands, and holds the last
/// picture across an epoch bump rather than flashing the glyph. Its longer edge
/// is capped at ~28 logical px (2× for crispness), fit into a small 30×17 box.
class _FootageThumbnail extends StatefulWidget {
  final AppStateStub app;
  final String itemId;
  final int epoch;
  final Widget placeholder;
  const _FootageThumbnail({
    super.key,
    required this.app,
    required this.itemId,
    required this.epoch,
    required this.placeholder,
  });

  @override
  State<_FootageThumbnail> createState() => _FootageThumbnailState();
}

class _FootageThumbnailState extends State<_FootageThumbnail> {
  static const double _w = 30;
  static const double _h = 17;
  static const int _maxEdge = 56; // ~28 logical px at 2×

  ui.Image? _image;
  int _loadedEpoch = -1;
  bool _loading = false;

  @override
  void initState() {
    super.initState();
    _maybeLoad();
  }

  @override
  void didUpdateWidget(_FootageThumbnail old) {
    super.didUpdateWidget(old);
    // Re-decode when the item changed or the document epoch advanced (a relink
    // or any edit); the held image stays on screen until the new one lands.
    if (old.itemId != widget.itemId || old.epoch != widget.epoch) {
      _maybeLoad();
    }
  }

  void _maybeLoad() {
    if (_loading || _loadedEpoch == widget.epoch) return;
    _loading = true;
    final epoch = widget.epoch;
    final itemId = widget.itemId;
    // The decode rides the render worker isolate (TF round 5): a cold video
    // thumbnail is a whole FFI decode under the bridge lock, and a microtask
    // alone never leaves the UI isolate — the per-row jank the tester felt.
    // The microtask remains only to keep the request itself off the build; the
    // no-worker fallback then decodes inline exactly as before.
    Future<void>(() {
      widget.app.requestThumbnail(itemId, _maxEdge, (frame) {
        if (!mounted || widget.epoch != epoch || widget.itemId != itemId) {
          _loading = false;
          return;
        }
        if (frame == null || frame.width == 0 || frame.height == 0) {
          _loading = false;
          _loadedEpoch = epoch; // don't hammer a footage item with no thumbnail
          return;
        }
        ui.decodeImageFromPixels(
          frame.rgba,
          frame.width,
          frame.height,
          ui.PixelFormat.rgba8888,
          (img) {
            _loading = false;
            _loadedEpoch = epoch;
            if (!mounted || widget.epoch != epoch || widget.itemId != itemId) {
              img.dispose();
              return;
            }
            setState(() {
              _image?.dispose();
              _image = img;
            });
          },
        );
      });
    });
  }

  @override
  void dispose() {
    _image?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final image = _image;
    if (image == null) {
      return SizedBox(
          width: _w, height: _h, child: Center(child: widget.placeholder));
    }
    return SizedBox(
      width: _w,
      height: _h,
      child: ClipRRect(
        borderRadius: BorderRadius.circular(2),
        child: Container(
          color: t.surface0,
          child: RawImage(image: image, fit: BoxFit.contain),
        ),
      ),
    );
  }
}

/// The floating label shown under the pointer while a footage row is dragged.
class _DragFeedback extends StatelessWidget {
  final String name;
  const _DragFeedback({required this.name});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            lumitIcon(LumitIcon.footage, size: 13, color: t.layer.footage),
            const SizedBox(width: 6),
            Text(name, style: t.small),
          ],
        ),
      ),
    );
  }
}

/// The actions the project context menu can raise (the egui project row menu,
/// panels.rs:909). All are now wired to real v0.5 ops; the item set matches
/// egui (Relink and Find missing footage on footage rows, Composition settings
/// on comps).
enum _ProjectMenuAction {
  compSettings,
  relink,
  findMissing,
  moveToRoot,
  delete,
}

/// Show the project context menu at [position] and run the chosen action.
Future<void> showProjectContextMenu({
  required BuildContext context,
  required AppStateStub app,
  required BridgeItem item,
  required bool missing,
  required Offset position,
  required VoidCallback onFindMissing,
  required Future<void> Function() onRelink,
}) async {
  final isComp = item.kind == BridgeItemKind.composition;
  final isFootage = item.kind == BridgeItemKind.footage;
  final action = await showLumitPopup<_ProjectMenuAction>(
    context: context,
    position: position,
    builder: (close) => FloatSurface(
      width: 210,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (isComp)
            MenuRow(
              onPressed: () => close(_ProjectMenuAction.compSettings),
              child: const Text('Composition settings…'),
            ),
          // Relink is offered on a missing footage row (egui: footage && missing).
          if (isFootage && missing)
            MenuRow(
              onPressed: () => close(_ProjectMenuAction.relink),
              child: const Text('Relink…'),
            ),
          // Find missing is offered on any footage row (egui panels.rs:923).
          if (isFootage)
            MenuRow(
              onPressed: () => close(_ProjectMenuAction.findMissing),
              child: const Text('Find missing footage'),
            ),
          MenuRow(
            onPressed: () => close(_ProjectMenuAction.moveToRoot),
            child: const Text('Move to root'),
          ),
          MenuRow(
            onPressed: () => close(_ProjectMenuAction.delete),
            child: const Text('Delete'),
          ),
        ],
      ),
    ),
  );
  if (action == null) return;
  if (!context.mounted) return;
  switch (action) {
    case _ProjectMenuAction.compSettings:
      if (isComp) {
        app.frontCompSelect(item.id);
        await showCompositionSettingsDialog(context, app);
      } else {
        app.setNotice('Composition settings apply to a composition');
      }
    case _ProjectMenuAction.relink:
      await onRelink();
    case _ProjectMenuAction.findMissing:
      onFindMissing();
    case _ProjectMenuAction.moveToRoot:
      app.moveToRoot(item.id);
    case _ProjectMenuAction.delete:
      // The egui Delete has no confirm (panels.rs:935) — it is one undo step.
      app.deleteItem(item.id);
  }
}
