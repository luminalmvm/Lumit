// The in-window menu bar (Windows; docs/07-UI-SPEC). Item set ported
// verbatim from shell/app_update.rs — File, Edit, Composition, Window.
// Engine-backed items dispatch to the stub and surface an honest notice.

import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import '../state/app_state.dart';
import '../state/settings.dart';
import '../state/workspace.dart';
import '../widgets/controls.dart';

class LumitMenuBar extends StatelessWidget {
  final AppStateStub app;
  final Workspace workspace;
  final VoidCallback onOpenSettings;
  final VoidCallback onOpenPalette;

  const LumitMenuBar({
    super.key,
    required this.app,
    required this.workspace,
    required this.onOpenSettings,
    required this.onOpenPalette,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      height: 26,
      color: t.surface2,
      child: Row(
        children: [
          const SizedBox(width: 4),
          _menu(context, 'File', [
            _Item('New project', app.newProject),
            _Item('Open project…', app.openProject),
            _Item('Import footage…', app.importFootage),
            _Item('Save', app.save),
            _Item.divider(),
            _Item('Export comp…',
                () => app.engine('Export comp (${workspace.export.defaultPreset.label})')),
            _Item.submenu('Export preset', [
              for (final p in ExportPreset.values)
                _Item(p.label, () => app.engine('Export (${p.label})')),
            ]),
            _Item.submenu('Export for sharing', [
              _Item('Discord 50 MB', () => app.engine('Share export 50 MB')),
              _Item('Small 10 MB', () => app.engine('Share export 10 MB')),
            ]),
          ]),
          _menu(context, 'Edit', [
            _Item('Undo', app.canUndo ? app.undo : null),
            _Item('Redo', app.canRedo ? app.redo : null),
          ]),
          _menu(context, 'Composition', [
            _Item('New composition', app.newComposition),
            _Item('Add solid layer', () => app.engine('Add solid layer')),
            _Item('Add text layer', () => app.engine('Add text layer')),
            _Item('Add camera layer', () => app.engine('Add camera layer')),
            _Item('Add adjustment layer', () => app.engine('Add adjustment layer')),
            _Item('Add sequence layer', () => app.engine('Add sequence layer')),
            _Item.divider(),
            _Item('Cut clip at playhead', () => app.engine('Cut clip at playhead')),
            _Item('Delete clip at playhead', () => app.engine('Delete clip at playhead')),
            _Item('Add marker at playhead', () => app.engine('Add marker at playhead')),
            _Item.submenu('Detect beats', [
              _Item('Detect', () => app.engine('Detect beats (sensitivity ${app.beatSensitivity})')),
            ]),
            _Item('Clear beat markers', () => app.engine('Clear beat markers')),
            _Item.submenu('Add mask', [
              _Item('Rectangle', () => app.engine('Add mask: rectangle')),
              _Item('Ellipse', () => app.engine('Add mask: ellipse')),
              _Item('Star', () => app.engine('Add mask: star')),
            ]),
            _Item.divider(),
            _Item('Composition settings…', () => app.engine('Composition settings')),
          ]),
          _menu(context, 'Window', [
            _Item('Command palette…', onOpenPalette),
            _Item('Reset workspace', workspace.resetWorkspaceLayout),
            _Item.divider(),
            _Item('Settings…', onOpenSettings),
          ]),
        ],
      ),
    );
  }

  Widget _menu(BuildContext context, String title, List<_Item> items) =>
      _MenuButton(title: title, items: items);
}

class _Item {
  final String? label;
  final VoidCallback? onPressed;
  final List<_Item>? children;
  final bool isDivider;

  _Item(this.label, this.onPressed)
      : children = null,
        isDivider = false;
  _Item.submenu(this.label, this.children)
      : onPressed = null,
        isDivider = false;
  _Item.divider()
      : label = null,
        onPressed = null,
        children = null,
        isDivider = true;
}

class _MenuButton extends StatelessWidget {
  final String title;
  final List<_Item> items;
  const _MenuButton({required this.title, required this.items});

  @override
  Widget build(BuildContext context) {
    return HouseButton(
      frameless: true,
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
      onPressed: () => _open(context),
      child: Text(title),
    );
  }

  void _open(BuildContext context) {
    final box = context.findRenderObject()! as RenderBox;
    final origin = box.localToGlobal(Offset(0, box.size.height));
    showLumitPopup<void>(
      context: context,
      position: origin,
      builder: (close) => _MenuList(items: items, close: () => close(null)),
    );
  }
}

class _MenuList extends StatelessWidget {
  final List<_Item> items;
  final VoidCallback close;
  const _MenuList({required this.items, required this.close});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      width: 230,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final item in items)
            if (item.isDivider)
              Container(
                height: 1,
                margin: const EdgeInsets.symmetric(vertical: 4),
                color: t.hairline,
              )
            else if (item.children != null)
              _SubmenuRow(item: item, closeAll: close)
            else
              MenuRow(
                onPressed: item.onPressed == null
                    ? close
                    : () {
                        close();
                        item.onPressed!();
                      },
                child: Text(
                  item.label!,
                  style: item.onPressed == null
                      ? t.body.copyWith(color: t.textDisabled)
                      : null,
                ),
              ),
        ],
      ),
    );
  }
}

class _SubmenuRow extends StatefulWidget {
  final _Item item;
  final VoidCallback closeAll;
  const _SubmenuRow({required this.item, required this.closeAll});

  @override
  State<_SubmenuRow> createState() => _SubmenuRowState();
}

class _SubmenuRowState extends State<_SubmenuRow> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        MenuRow(
          onPressed: () => setState(() => _open = !_open),
          child: Row(
            children: [
              Expanded(child: Text(widget.item.label!)),
              lumitIcon(
                _open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                size: 10,
                color: t.textMuted,
              ),
            ],
          ),
        ),
        if (_open)
          Padding(
            padding: const EdgeInsets.only(left: 14),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final c in widget.item.children!)
                  MenuRow(
                    onPressed: () {
                      widget.closeAll();
                      c.onPressed?.call();
                    },
                    child: Text(c.label!),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}
