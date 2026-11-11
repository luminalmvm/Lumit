part of 'effects_presets_panel_frb.dart';

// The Graphics templates group of Effects & presets: the saved templates as
// tiles, as many across as fit, under folding group headings. A tile shows the
// template's poster, and scrubs its strip of frames as the pointer crosses it.
// Dragged, it carries a `TemplateDragData` onto either timeline or the Project
// panel; double-clicked, or `Enter` on the picked tile, it is placed at the
// playhead.

/// How wide a tile is drawn at most, before another fits across.
const double _tileWidth = 132;

/// The room between two tiles, across and down.
const double _tileGap = 8;

/// A template's poster and strip are kept by its file and its version, so a
/// newer version of it is read afresh.
String _tileKey(BridgeLibraryTemplate t) => '${t.path}|${t.version}';

/// PNG or JPEG bytes as a picture, or null when there are none or they do
/// not decode.
Future<ui.Image?> _decodeTile(Uint8List bytes) async {
  if (bytes.isEmpty) return null;
  try {
    final codec = await ui.instantiateImageCodec(bytes);
    final frame = await codec.getNextFrame();
    codec.dispose();
    return frame.image;
  } catch (_) {
    return null;
  }
}

mixin _TemplateShelf on State<EffectsPresetsPanelFrb> {
  // What the panel has that the shelf uses.
  Set<String> get _shut;
  bool _isOpen(String key, String needle);
  void _star(LumitUiState ui, String key);
  Widget _heading(
    LumitTheme t, {
    required String group,
    required String label,
    required bool open,
    int depth = 0,
  });

  /// The library, read with the presets and after each change to it.
  List<BridgeLibraryTemplate> _templates = const [];

  /// The picked tile, by its file.
  String? _pickedTile;

  /// The shell state the keys are heard through, and the Delete claim this
  /// panel's stands in front of.
  LumitUiState? _keysUi;
  bool Function()? _priorDelete;

  /// The posters and strips read so far, by [_tileKey]. Null while one is
  /// being read, or for a file that has none.
  final Map<String, ui.Image?> _posters = {};
  final Map<String, ui.Image?> _strips = {};

  /// The tiles drawn this build, each grid's in order, and how many stand
  /// across in each: what the arrow keys walk.
  final List<List<BridgeLibraryTemplate>> _tileGrids = [];
  final Map<int, int> _gridColumns = {};

  /// The Graphics templates heading's name in the folds.
  static const String _templatesKey = '*templates';

  /// A template group's name in the folds is its path after this.
  static const String _templateGroupKey = '*template-group:';

  void _readTemplates() {
    try {
      _templates = widget.presetsLister == null ? listTemplates() : const [];
    } catch (_) {
      _templates = const [];
    }
    // What the library no longer holds is let go.
    final keep = {for (final t in _templates) _tileKey(t)};
    for (final held in [_posters, _strips]) {
      held.removeWhere((key, picture) {
        if (keep.contains(key)) return false;
        picture?.dispose();
        return true;
      });
    }
    if (!_templates.any((t) => t.path == _pickedTile)) _pickedTile = null;
  }

  /// Hear the tiles' keys: the arrows, `Enter` and `F2` on the keyboard,
  /// which every handler hears, and `Delete` through the shell's claim.
  void _bindTileKeys(LumitUiState ui) {
    _keysUi = ui;
    HardwareKeyboard.instance.addHandler(_onTileKey);
    _priorDelete = ui.deleteClaim;
    ui.deleteClaim = _deleteClaim;
  }

  void _disposeTemplates() {
    HardwareKeyboard.instance.removeHandler(_onTileKey);
    final ui = _keysUi;
    if (ui != null && ui.deleteClaim == _deleteClaim) {
      ui.deleteClaim = _priorDelete;
    }
    for (final picture in [..._posters.values, ..._strips.values]) {
      picture?.dispose();
    }
  }

  /// Every template group, and every group above one.
  Set<String> get _templateGroups => {
        for (final t in _templates)
          ..._EffectsPresetsPanelFrbState._withParents(t.group),
      };

  /// The fold keys of the Graphics templates heading and its groups, for
  /// Collapse all.
  Iterable<String> get _templateFoldKeys => [
        _templatesKey,
        for (final g in _templateGroups) '$_templateGroupKey$g',
      ];

  bool _templateMatches(BridgeLibraryTemplate t, String needle) =>
      needle.isEmpty ||
      t.name.toLowerCase().contains(needle) ||
      t.controls.any((c) => c.toLowerCase().contains(needle));

  bool _isFavouriteTemplate(LumitUiState ui, BridgeLibraryTemplate t) =>
      ui.workspace.isFavouriteEffect(templateFavouriteKey(t));

  bool _anyFavouriteTemplate(LumitUiState ui) =>
      _templates.any((t) => _isFavouriteTemplate(ui, t));

  /// The starred templates a search leaves, as one grid for Favourites.
  List<BridgeLibraryTemplate> _favouriteTemplates(
          LumitUiState ui, String needle) =>
      [
        for (final t in _templates)
          if (_isFavouriteTemplate(ui, t) && _templateMatches(t, needle)) t,
      ];

  /// Read a tile's poster, once.
  void _askPoster(BridgeLibraryTemplate t) {
    final key = _tileKey(t);
    if (_posters.containsKey(key)) return;
    _posters[key] = null;
    () async {
      ui.Image? picture;
      try {
        picture = await _decodeTile(await templatePoster(path: t.path));
      } catch (_) {}
      if (!mounted || !_posters.containsKey(key)) {
        picture?.dispose();
        return;
      }
      setState(() => _posters[key] = picture);
    }();
  }

  /// Read a tile's strip of frames, once, the first time the pointer is
  /// over it.
  void _askStrip(BridgeLibraryTemplate t) {
    final key = _tileKey(t);
    if (_strips.containsKey(key)) return;
    _strips[key] = null;
    () async {
      ui.Image? picture;
      try {
        picture = await _decodeTile(await templatePreview(path: t.path));
      } catch (_) {}
      if (!mounted || !_strips.containsKey(key)) {
        picture?.dispose();
        return;
      }
      setState(() => _strips[key] = picture);
    }();
  }

  /// The Graphics templates heading, its groups, and the tiles in them.
  List<Widget> _templateRows(LumitTheme t, LumitUiState ui, String needle) {
    final shown = _templates.where((x) => _templateMatches(x, needle)).toList();
    if (shown.isEmpty) return const [];
    final groups = <String>{
      for (final x in shown)
        ..._EffectsPresetsPanelFrbState._withParents(x.group),
    };
    final open = _isOpen(_templatesKey, needle);
    return [
      _heading(t,
          group: _templatesKey, label: l10n.graphicsTemplates, open: open),
      if (open) ..._templateGroupRows(t, ui, needle, '', groups, shown),
    ];
  }

  /// The groups directly inside [parent], each with what is in it, then the
  /// tiles in [parent] itself.
  List<Widget> _templateGroupRows(LumitTheme t, LumitUiState ui, String needle,
      String parent, Set<String> groups, List<BridgeLibraryTemplate> shown) {
    final depth = parent.isEmpty ? 0 : '/'.allMatches(parent).length + 1;
    final inside = groups
        .where((g) => _EffectsPresetsPanelFrbState._parentOf(g) == parent)
        .toList()
      ..sort((a, b) => a.toLowerCase().compareTo(b.toLowerCase()));
    final here = [
      for (final x in shown)
        if (x.group == parent) x,
    ];
    return [
      for (final group in inside) ...[
        _heading(t,
            group: '$_templateGroupKey$group',
            label: group.substring(group.lastIndexOf('/') + 1),
            open: _isOpen('$_templateGroupKey$group', needle),
            depth: depth + 1),
        if (_isOpen('$_templateGroupKey$group', needle))
          ..._templateGroupRows(t, ui, needle, group, groups, shown),
      ],
      if (here.isNotEmpty)
        _tileGrid(ui, here, indent: depth * _EffectsPresetsPanelFrbState._indent),
    ];
  }

  /// [templates] as tiles, as many across as fit and never fewer than two.
  Widget _tileGrid(LumitUiState ui, List<BridgeLibraryTemplate> templates,
      {double indent = 0}) {
    final grid = _tileGrids.length;
    _tileGrids.add(templates);
    final scrub =
        ThemeScope.of(context).animationLevel != AnimationLevel.none;
    return Padding(
      padding: EdgeInsetsDirectional.fromSTEB(
          _starColumn + indent, 4, 10, 6),
      child: LayoutBuilder(builder: (context, box) {
        final across = math.max(
            2, ((box.maxWidth + _tileGap) / (_tileWidth + _tileGap)).floor());
        _gridColumns[grid] = across;
        final width = math.max(
            1.0, (box.maxWidth - _tileGap * (across - 1)) / across);
        return Wrap(
          spacing: _tileGap,
          runSpacing: _tileGap,
          children: [
            for (final template in templates)
              _TemplateTile(
                key: ValueKey<String>('template-tile-${template.path}'),
                template: template,
                width: width,
                poster: _posters[_tileKey(template)],
                strip: scrub ? _strips[_tileKey(template)] : null,
                picked: template.path == _pickedTile,
                favourite: _isFavouriteTemplate(ui, template),
                onShown: () => _askPoster(template),
                onHover: scrub ? () => _askStrip(template) : null,
                onPick: () => _pickTile(template),
                onPlace: () => _placeTile(ui, template),
                onMenu: (at) => _tileMenu(at, ui, template),
                onToggleFavourite: () =>
                    _star(ui, templateFavouriteKey(template)),
              ),
          ],
        );
      }),
    );
  }

  void _pickTile(BridgeLibraryTemplate template) {
    // A text field let go of, so the tiles' keys reach them.
    FocusManager.instance.primaryFocus?.unfocus();
    setState(() => _pickedTile = template.path);
  }

  Future<void> _placeTile(
      LumitUiState ui, BridgeLibraryTemplate template) async {
    final app = Provider.of<LumitState>(context, listen: false);
    await placeTemplateAtPlayhead(app, ui, template.path);
  }

  /// The picked tile's template, if it is still drawn.
  BridgeLibraryTemplate? get _picked {
    for (final grid in _tileGrids) {
      for (final t in grid) {
        if (t.path == _pickedTile) return t;
      }
    }
    return null;
  }

  /// The picked tile, while Effects & presets has the keys and no field or
  /// window has them first.
  BridgeLibraryTemplate? get _keyedTile {
    final ui = _keysUi;
    if (!mounted || ui == null || lumitModalOpen) return null;
    if (ui.activePanel != Panel.effectsAndPresets) return null;
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return null;
    }
    return _picked;
  }

  /// The tiles' keys: the arrows move the pick, `Enter` places it and `F2`
  /// renames it. No modifier, so the app's own chords pass by.
  bool _onTileKey(KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) return false;
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed || keys.isMetaPressed || keys.isAltPressed ||
        keys.isShiftPressed) {
      return false;
    }
    final picked = _keyedTile;
    final ui = _keysUi;
    if (picked == null || ui == null) return false;
    final key = event.logicalKey;
    if (key == LogicalKeyboardKey.enter ||
        key == LogicalKeyboardKey.numpadEnter) {
      if (event is KeyDownEvent) _placeTile(ui, picked);
      return true;
    }
    if (key == LogicalKeyboardKey.f2) {
      if (event is KeyDownEvent) _renameTemplate(ui, picked);
      return true;
    }
    final (across, down) = switch (key) {
      LogicalKeyboardKey.arrowLeft => (-1, 0),
      LogicalKeyboardKey.arrowRight => (1, 0),
      LogicalKeyboardKey.arrowUp => (0, -1),
      LogicalKeyboardKey.arrowDown => (0, 1),
      _ => (0, 0),
    };
    if (across == 0 && down == 0) return false;
    final next = _tileStep(across, down);
    if (next != null) setState(() => _pickedTile = next.path);
    return true;
  }

  /// `Delete` deletes the picked tile's template, after asking.
  bool _deleteClaim() {
    final picked = _keyedTile;
    final ui = _keysUi;
    if (picked == null || ui == null) return _priorDelete?.call() ?? false;
    _deleteTemplate(ui, picked);
    return true;
  }

  /// The tile [across] along or [down] rows from the picked one, through
  /// every grid drawn as if they were one. Null at an edge.
  BridgeLibraryTemplate? _tileStep(int across, int down) {
    final at = <(int, int)>[];
    for (var g = 0; g < _tileGrids.length; g++) {
      for (var i = 0; i < _tileGrids[g].length; i++) {
        if (_tileGrids[g][i].path == _pickedTile) at.add((g, i));
      }
    }
    if (at.isEmpty) return null;
    final (g, i) = at.first;
    final grid = _tileGrids[g];
    if (across != 0) {
      final flat = [for (final x in _tileGrids) ...x];
      final here = flat.indexWhere((x) => x.path == _pickedTile);
      final to = here + across;
      return to < 0 || to >= flat.length ? null : flat[to];
    }
    final cols = _gridColumns[g] ?? 2;
    final column = i % cols;
    final to = i + down * cols;
    if (to >= 0 && to < grid.length) return grid[to];
    // Down from the last row of a grid with a gap under the pick lands on
    // its last tile.
    if (down > 0 && i ~/ cols < (grid.length - 1) ~/ cols) return grid.last;
    final other = g + down;
    if (other < 0 || other >= _tileGrids.length) return null;
    final next = _tileGrids[other];
    final otherCols = _gridColumns[other] ?? 2;
    final row = down > 0 ? 0 : (next.length - 1) ~/ otherCols;
    return next[math.min(row * otherCols + column, next.length - 1)];
  }

  /// The menu on a tile.
  void _tileMenu(Offset at, LumitUiState ui, BridgeLibraryTemplate template) {
    _pickTile(template);
    final app = Provider.of<LumitState>(context, listen: false);
    final favourite = _isFavouriteTemplate(ui, template);
    final forTitle = ui.workspace.titleTemplate == template.path;
    final groups = _templateGroups.toList()
      ..sort((a, b) => a.toLowerCase().compareTo(b.toLowerCase()));
    showMenuAt<void>(
      context: context,
      position: at,
      width: 220,
      rows: (close) {
        void run(void Function() act) {
          close(null);
          act();
        }

        return [
          MenuRow(
            key: const ValueKey('template-place'),
            onPressed: () => run(() => _placeTile(ui, template)),
            child: MenuLabel(l10n.templatePlace, ticked: false),
          ),
          MenuRow(
            key: const ValueKey('template-favourite'),
            onPressed: () =>
                run(() => _star(ui, templateFavouriteKey(template))),
            child: MenuLabel(l10n.tipFavourite, ticked: favourite),
          ),
          MenuRow(
            key: const ValueKey('template-for-title'),
            onPressed: () => run(() => ui.workspace
                .setTitleTemplate(forTitle ? '' : template.path)),
            child: MenuLabel(l10n.templateUseForTitle, ticked: forTitle),
          ),
          MenuRow(
            key: const ValueKey('template-open'),
            onPressed: () => run(() => openTemplate(app, ui, template.path)),
            child: MenuLabel(l10n.templateOpen, ticked: false),
          ),
          const MenuDivider(),
          MenuRow(
            key: const ValueKey('template-rename'),
            onPressed: () => run(() => _renameTemplate(ui, template)),
            child: MenuLabel(l10n.renameEllipsis,
                action: 'item.rename', ticked: false),
          ),
          SubmenuRow(
            key: const ValueKey('template-move-to'),
            closeParent: () => close(null),
            submenu: (dismiss) => FloatSurface(
              width: 220,
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxHeight: 400),
                child: SingleChildScrollView(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      for (final group in ['', ...groups])
                        MenuRow(
                          key: ValueKey<String>('template-move-to-$group'),
                          selected: group == template.group,
                          onPressed: () {
                            dismiss();
                            _moveTemplate(ui, template, group);
                          },
                          child: Padding(
                            padding: EdgeInsetsDirectional.only(
                                start: group.isEmpty
                                    ? 0
                                    : ('/'.allMatches(group).length) *
                                        _EffectsPresetsPanelFrbState._indent),
                            child: Text(
                                group.isEmpty
                                    ? l10n.graphicsTemplates
                                    : group.substring(
                                        group.lastIndexOf('/') + 1),
                                overflow: TextOverflow.ellipsis),
                          ),
                        ),
                      const MenuDivider(),
                      MenuRow(
                        key: const ValueKey('template-move-to-new'),
                        onPressed: () {
                          dismiss();
                          _moveToNewGroup(ui, template);
                        },
                        child: Text(l10n.templateLibraryNewGroup),
                      ),
                    ],
                  ),
                ),
              ),
            ),
            child: MenuLabel(l10n.presetMoveTo, ticked: false),
          ),
          MenuRow(
            key: const ValueKey('template-reveal'),
            onPressed: () => run(() => revealInFolder(path: template.path)),
            child: MenuLabel(l10n.projectShowInFolder, ticked: false),
          ),
          const MenuDivider(),
          MenuRow(
            key: const ValueKey('template-delete'),
            onPressed: () => run(() => _deleteTemplate(ui, template)),
            child: MenuLabel(l10n.delete,
                action: 'edit.delete.selection', ticked: false),
          ),
        ];
      },
    );
  }

  /// A template's file moved: the pick and Insert title's choice follow it.
  void _moved(LumitUiState ui, String from, String to) {
    if (ui.workspace.titleTemplate == from) ui.workspace.setTitleTemplate(to);
    if (_pickedTile == from) _pickedTile = to;
    ui.presetsChanged.value++;
  }

  Future<void> _renameTemplate(
      LumitUiState ui, BridgeLibraryTemplate template) async {
    final name = await askThemeName(context,
        title: l10n.rename, suggested: template.name, confirm: l10n.rename);
    if (name == null || name == template.name || !mounted) return;
    try {
      _moved(ui, template.path,
          await renameTemplate(path: template.path, name: name));
    } catch (_) {
      // A name already taken, or a file that has gone: nothing changes.
      ui.presetsChanged.value++;
    }
  }

  void _moveTemplate(
      LumitUiState ui, BridgeLibraryTemplate template, String group) {
    if (group == template.group) return;
    try {
      _moved(ui, template.path,
          moveTemplate(path: template.path, group: group));
    } catch (_) {
      ui.presetsChanged.value++;
    }
  }

  Future<void> _moveToNewGroup(
      LumitUiState ui, BridgeLibraryTemplate template) async {
    final name = await askThemeName(context,
        title: l10n.templateLibraryNewGroup, suggested: '', confirm: l10n.create);
    if (name == null || name.trim().isEmpty || !mounted) return;
    _moveTemplate(ui, template, name.trim());
    // Open where it went, so it shows.
    setState(() => _shut
      ..remove(_templatesKey)
      ..remove('$_templateGroupKey${name.trim()}'));
  }

  /// Delete a template's file, asking first as it can't be undone. Projects
  /// that placed it keep theirs.
  Future<void> _deleteTemplate(
      LumitUiState ui, BridgeLibraryTemplate template) async {
    if (!await askBefore(context,
        l10n.templateDeleteQuestion(template.name), l10n.delete)) {
      return;
    }
    try {
      deleteTemplate(path: template.path);
    } catch (_) {}
    if (ui.workspace.titleTemplate == template.path) {
      ui.workspace.setTitleTemplate('');
    }
    if (_isFavouriteTemplate(ui, template) &&
        !_templates.any((t) => t.id == template.id && t.path != template.path)) {
      ui.workspace.toggleFavouriteEffect(templateFavouriteKey(template));
    }
    ui.presetsChanged.value++;
  }
}

/// One template in the library as a tile: its poster in a 16:9 cell, its
/// name under it. Moving the pointer across it scrubs its strip of frames.
class _TemplateTile extends StatefulWidget {
  final BridgeLibraryTemplate template;
  final double width;
  final ui.Image? poster;

  /// Its frames side by side, or null while unread, with none, or with
  /// Motion off.
  final ui.Image? strip;
  final bool picked;
  final bool favourite;

  /// Asks for its poster, as it first comes on screen.
  final VoidCallback onShown;

  /// Asks for its strip, the first time the pointer is over it. Null when
  /// it does not scrub.
  final VoidCallback? onHover;
  final VoidCallback onPick;
  final VoidCallback onPlace;
  final void Function(Offset at) onMenu;
  final VoidCallback onToggleFavourite;

  const _TemplateTile({
    super.key,
    required this.template,
    required this.width,
    required this.poster,
    required this.strip,
    required this.picked,
    required this.favourite,
    required this.onShown,
    required this.onHover,
    required this.onPick,
    required this.onPlace,
    required this.onMenu,
    required this.onToggleFavourite,
  });

  @override
  State<_TemplateTile> createState() => _TemplateTileState();
}

class _TemplateTileState extends State<_TemplateTile> {
  /// Where the pointer is across the poster, 0 to 1, or null when it is
  /// not over it.
  double? _scrub;
  bool _over = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) widget.onShown();
    });
  }

  @override
  void didUpdateWidget(covariant _TemplateTile old) {
    super.didUpdateWidget(old);
    if (old.template.version != widget.template.version ||
        old.template.path != widget.template.path) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) widget.onShown();
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final template = widget.template;
    final cellHeight = widget.width * 9 / 16;
    final strip = widget.strip;
    final shape = template.height > 0 ? template.width / template.height : 16 / 9;
    // The strip's frames are each as wide as it is tall times the
    // template's shape.
    final frames = strip == null
        ? 0
        : math.max(1, (strip.width / (strip.height * shape)).round());
    final scrubbing = strip != null && _scrub != null;
    final frame = scrubbing
        ? math.min(frames - 1, (_scrub! * frames).floor())
        : 0;
    final cell = MouseRegion(
      onEnter: (_) {
        widget.onHover?.call();
        setState(() => _over = true);
      },
      onHover: (e) => setState(
          () => _scrub = (e.localPosition.dx / widget.width).clamp(0.0, 1.0)),
      onExit: (_) => setState(() {
        _scrub = null;
        _over = false;
      }),
      child: SizedBox(
        width: widget.width,
        height: cellHeight,
        child: Stack(
          children: [
            Positioned.fill(
              child: Container(
                decoration: BoxDecoration(
                  color: t.surface0,
                  borderRadius: BorderRadius.circular(t.tokens.wellRadius),
                  border: Border.all(
                    color: widget.picked
                        ? t.accent
                        : _over
                            ? t.hairlineStrong
                            : t.hairline,
                    width: widget.picked ? 1.5 : 1,
                  ),
                ),
                clipBehavior: Clip.antiAlias,
                child: CustomPaint(
                  painter: _TileFramePainter(
                    picture: scrubbing ? strip : widget.poster,
                    frame: scrubbing ? frame : null,
                    frames: frames,
                  ),
                ),
              ),
            ),
            // How far through the strip the pointer is.
            if (scrubbing)
              PositionedDirectional(
                start: 1,
                bottom: 1,
                width: math.max(2, (widget.width - 2) * _scrub!),
                height: 2,
                child: ColoredBox(color: t.textPrimary),
              ),
            if (widget.favourite || _over)
              PositionedDirectional(
                top: 2,
                end: 2,
                child: LumitTooltip(
                  message: widget.favourite
                      ? l10n.tipUnfavourite
                      : l10n.tipFavourite,
                  child: HoverMark(
                    builder: (context, lit) => GestureDetector(
                      key: ValueKey<String>(
                          'template-star-${template.path}'),
                      behavior: HitTestBehavior.opaque,
                      onTap: widget.onToggleFavourite,
                      child: Padding(
                        padding: const EdgeInsets.all(3),
                        child: lumitIcon(
                          LumitIcon.star,
                          size: iconSize,
                          color: widget.favourite
                              ? t.accent
                              : Color.lerp(
                                  t.textMuted, t.textPrimary, lit)!,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
    final missing = template.missingFonts.isNotEmpty;
    final tile = GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTapDown: (_) => widget.onPick(),
      onDoubleTap: widget.onPlace,
      onSecondaryTapDown: (d) => widget.onMenu(d.globalPosition),
      child: SizedBox(
        width: widget.width,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            cell,
            const SizedBox(height: 3),
            Row(
              children: [
                Expanded(
                  child: Text(template.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: t.body.copyWith(
                          color: widget.picked ? t.textPrimary : null)),
                ),
                if (missing)
                  LumitTooltip(
                    message: l10n.templateMissingFonts(
                        template.missingFonts.join(', ')),
                    child: Padding(
                      padding: const EdgeInsetsDirectional.only(start: 4),
                      child: glyph.LumitIcon(LumitIcons.missing,
                          key: ValueKey<String>(
                              'template-missing-${template.path}'),
                          size: 12,
                          colour: t.warning),
                    ),
                  ),
              ],
            ),
          ],
        ),
      ),
    );
    return Draggable<TemplateDragData>(
      data: TemplateDragData(template.path, template.name,
          seconds: template.seconds),
      dragAnchorStrategy: pointerDragAnchorStrategy,
      feedback: FloatSurface(
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
          child: Text(template.name, style: t.small),
        ),
      ),
      child: tile,
    );
  }
}

/// A poster, or one frame of a strip, at the template's own shape and as
/// large as fits in the cell, the cell's ground showing round it.
class _TileFramePainter extends CustomPainter {
  final ui.Image? picture;

  /// The strip's frame to draw, or null to draw the whole picture.
  final int? frame;
  final int frames;

  _TileFramePainter({
    required this.picture,
    required this.frame,
    required this.frames,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final image = picture;
    if (image == null || size.isEmpty) return;
    final Rect source;
    if (frame case final f?) {
      final w = image.width / math.max(1, frames);
      source = Rect.fromLTWH(f * w, 0, w, image.height.toDouble());
    } else {
      source = Rect.fromLTWH(
          0, 0, image.width.toDouble(), image.height.toDouble());
    }
    final fit = source.width / source.height;
    final cell = size.width / size.height;
    final drawn = fit > cell
        ? Size(size.width, size.width / fit)
        : Size(size.height * fit, size.height);
    final target = Rect.fromCenter(
        center: size.center(Offset.zero),
        width: drawn.width,
        height: drawn.height);
    canvas.drawImageRect(
        image, source, target, Paint()..filterQuality = FilterQuality.medium);
  }

  @override
  bool shouldRepaint(_TileFramePainter old) =>
      old.picture != picture ||
      old.frame != frame ||
      old.frames != frames;
}
